//! Database reads through the application interface, against answers recorded on the
//! real site (`tests/fixtures/app_*.json`). A local fake serves them, and the web pages
//! the client falls back to, and records what is asked.

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
use parfumo_client::{ClientConfig, ClientError, Gender, GlobalLimiter, ParfumoClient, PerfumeDetails, ReviewSort, SearchQuery, SearchSort};
use serde_json::{Value, json};

const PERFUMES_BRAND: &str = include_str!("fixtures/app_perfumes_brand.json");
const PERFUMES_TEXT: &str = include_str!("fixtures/app_perfumes_text.json");
const SEARCH_CREED: &str = include_str!("fixtures/app_search_creed.json");
const PERFUME_DETAILS: &str = include_str!("fixtures/app_perfume_details.json");
/// What the interface answers for reviews and statements: served by the fake, and never asked.
const REVIEWS: &str = include_str!("fixtures/app_reviews.json");
const STATEMENTS: &str = include_str!("fixtures/app_statements.json");
/// The web site: the page of a perfume the Creed answers of the interface do not hold,
/// and its fragments.
const WEB_PERFUME: &str = include_str!("fixtures/perfume_sauvage.html");
const WEB_CLASSIFICATION: &str = include_str!("fixtures/frag_classification_pie.html");
const WEB_REVIEWS: &str = include_str!("fixtures/frag_reviews_full.html");
const WEB_STATEMENTS: &str = include_str!("fixtures/frag_statements.html");

const SAUVAGE: &str = "Dior/Sauvage_Eau_de_Toilette";
/// A number the interface answers 404 to.
const UNKNOWN: u64 = 404_404;
/// A number answered with a refusal in JSON under status 200, as the interface does for a
/// brand it does not have (`docs/APP_API.md`); what it answers for a perfume was not recorded.
const REFUSED: u64 = 500_500;

#[derive(Default)]
struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// What `/perfumes` answers instead of the recorded listings.
    perfumes: Mutex<Option<String>>,
    /// The record a number answers instead of the recorded one.
    records: Mutex<HashMap<u64, String>>,
    /// Whether a page of the web site was asked with the session token.
    token_on_web: AtomicBool,
}

impl Site {
    /// What was asked since the last look.
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

/// The recorded record, changed by `change`.
fn record(change: impl FnOnce(&mut Value)) -> String {
    let mut record: Value = serde_json::from_str(PERFUME_DETAILS).unwrap();
    change(&mut record);
    record.to_string()
}

/// The recorded record as the site gives it for another perfume of the recorded listing of
/// the brand: under its number, with its slug.
fn record_of(number: u64) -> String {
    let listing: Value = serde_json::from_str(PERFUMES_BRAND).unwrap();
    let row = listing["rows"].as_array().unwrap().iter().find(|row| row["p_id"] == number.to_string().as_str()).cloned();
    record(|r| {
        r["p"]["id"] = number.to_string().into();
        if let Some(row) = row {
            r["p"]["name_url"] = row["p_name_url"].clone();
        }
    })
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked.clone());
    let json = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    let not_found = || (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response();
    let token = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if !uri.path().starts_with("/app/v1/") {
        site.token_on_web.fetch_or(token.is_some(), Ordering::SeqCst);
        let html = |text: &'static str| (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], text).into_response();
        return match uri.path() {
            "/api/alive" => json(r#"{"csrf_key":"fixturecsrfkey00"}"#),
            "/Perfumes/Dior/Sauvage_Eau_de_Toilette" => html(WEB_PERFUME),
            "/action/perfume/get_classification_chart.php" => html(WEB_CLASSIFICATION),
            "/action/perfume/get_reviews.php" => html(WEB_REVIEWS),
            "/action/perfume/get_statements.php" => html(WEB_STATEMENTS),
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
        "/app/v1/search" => json(SEARCH_CREED),
        "/app/v1/perfumes" => match site.perfumes.lock().unwrap().as_deref() {
            Some(answer) => json(answer),
            None if asked.contains("b_id=") => json(PERFUMES_BRAND),
            None => json(PERFUMES_TEXT),
        },
        path if path.starts_with("/app/v1/perfume_details/") => {
            let number: u64 = path.trim_start_matches("/app/v1/perfume_details/").parse().unwrap_or_default();
            match site.records.lock().unwrap().get(&number) {
                Some(answer) => json(answer),
                None if number == UNKNOWN => not_found(),
                None if number == REFUSED => json(r#"{"error":"Perfume not found"}"#),
                None => json(&record_of(number)),
            }
        }
        path if path.starts_with("/app/v1/reviews/") => json(REVIEWS),
        path if path.starts_with("/app/v1/statements/") => json(STATEMENTS),
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
        max_retries: 0,
        ..ClientConfig::default()
    };
    let client = ParfumoClient::new(cfg, GlobalLimiter::new(0), None).unwrap();
    client.app_login("someone@example.org", "pw").await.unwrap();
    site.take();
    (client, site)
}

fn by_brand(brand: &str) -> SearchQuery {
    SearchQuery { brand: Some(brand.into()), ..SearchQuery::default() }
}

fn by_year(year: u16) -> SearchQuery {
    SearchQuery { year: Some(year), ..SearchQuery::default() }
}

/// The shares of a group of the classification, in the order of the record.
fn shares<'a>(p: &'a PerfumeDetails, group: &str) -> Vec<(&'a str, f32)> {
    let group = p.classification.iter().find(|g| g.group == group).unwrap_or_else(|| panic!("no group {group}"));
    group.shares.iter().map(|s| (s.label.as_str(), s.percent)).collect()
}

/// The groups of the classification with their votes and their strongest share.
fn groups(p: &PerfumeDetails) -> Vec<(&str, Option<u32>, &str)> {
    p.classification.iter().map(|g| (g.group.as_str(), g.votes, g.shares[0].label.as_str())).collect()
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

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

    let asked = site.take();
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
    // A document of the index has the full name of the perfume and nothing else about its
    // concentration: none is given.
    assert!(page.items.iter().all(|p| p.concentration.is_none()));
}

#[tokio::test]
async fn words_ask_for_the_most_rated_first_and_an_unrated_perfume_has_no_rating() {
    let (c, site) = logged_in().await;
    let page = c.search_perfumes(&SearchQuery { text: Some(" sauvage ".into()), year: Some(2015), ..SearchQuery::default() }, 1).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/perfumes?page=1&per_page=20&filter=sauvage&y_0=2015&g_f=1&g_m=1&g_u=1&o=nr_desc"]);
    assert_eq!(page.total_items, Some(151));
    let unrated = page.items.iter().find(|p| p.name == "Ambre Sauvage").unwrap();
    assert_eq!((unrated.rating, unrated.rating_count, unrated.year), (None, None, None), "zeros of the site are absences");
}

#[tokio::test]
async fn a_search_always_asks_for_an_order() {
    let (c, site) = logged_in().await;
    // Without an order the interface lists by name: with no sort asked, a brand alone and a
    // year alone are asked the most rated first too, which is the order of the web search.
    c.search_perfumes(&by_brand("creed"), 1).await.unwrap();
    assert_eq!(site.take()[1], "/app/v1/perfumes?page=1&per_page=20&b_id=253&g_f=1&g_m=1&g_u=1&o=nr_desc");
    c.search_perfumes(&by_year(2020), 1).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/perfumes?page=1&per_page=20&y_0=2020&g_f=1&g_m=1&g_u=1&o=nr_desc"]);

    // Every order of the web search is passed on under its own code.
    for (sort, code) in [
        (SearchSort::NameAsc, "n_asc"),
        (SearchSort::NameDesc, "n_desc"),
        (SearchSort::Newest, "p_id_desc"),
        (SearchSort::PopularityDesc, "nr_desc"),
        (SearchSort::PopularityAsc, "nr_asc"),
        (SearchSort::YearDesc, "y_desc"),
        (SearchSort::YearAsc, "y_asc"),
        (SearchSort::ScentDesc, "v_desc"),
        (SearchSort::DurabilityDesc, "d_desc"),
        (SearchSort::SillageDesc, "s_desc"),
        (SearchSort::BottleDesc, "b_desc"),
        (SearchSort::PricingDesc, "p_desc"),
    ] {
        c.search_perfumes(&SearchQuery { sort, ..by_year(2020) }, 1).await.unwrap();
        assert_eq!(site.take(), [format!("/app/v1/perfumes?page=1&per_page=20&y_0=2020&g_f=1&g_m=1&g_u=1&o={code}")], "{sort:?}");
    }
}

#[tokio::test]
async fn a_brand_the_search_of_the_interface_does_not_find_is_left_to_the_web_search() {
    let (c, site) = logged_in().await;
    // The fake has no web search: the outcome is the one of the web code, which resolves
    // the brand its own way. What matters is that it is asked, and no listing of the interface.
    let _ = c.search_perfumes(&by_brand("Cre"), 1).await;
    let asked = site.take();
    assert_eq!(&asked[..3], ["/app/v1/search?filter=Cre", "/api/alive", "/action/s_x/label_filters_brands.php"], "{asked:?}");
    assert!(asked.iter().all(|a| !a.starts_with("/app/v1/perfumes")), "{asked:?}");
    assert!(!site.token_on_web.load(Ordering::SeqCst), "the session token is for the application interface only");
}

#[tokio::test]
async fn every_page_the_interface_counts_is_promised_and_a_page_past_the_end_is_empty() {
    let (c, site) = logged_in().await;
    // The web search stops at 1000 results; the interface serves the pages beyond.
    *site.perfumes.lock().unwrap() = Some(PERFUMES_TEXT.replace("\"matches\": 151", "\"matches\": 30000"));
    let far = c.search_perfumes(&by_year(2020), 60).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/perfumes?page=60&per_page=20&y_0=2020&g_f=1&g_m=1&g_u=1&o=nr_desc"], "asked of the interface past page 50");
    assert_eq!((far.items.len(), far.page, far.total_items, far.total_pages), (5, 60, Some(30000), Some(1500)));

    // A page that starts past the last match is empty, even if the site sends rows.
    *site.perfumes.lock().unwrap() = Some(PERFUMES_TEXT.replace("\"matches\": 151", "\"matches\": 5"));
    let past = c.search_perfumes(&by_year(2020), 2).await.unwrap();
    assert_eq!((past.items.len(), past.total_items, past.total_pages), (0, Some(5), Some(1)));

    // No match at all is an answer.
    *site.perfumes.lock().unwrap() = Some(r#"{"matches":0,"rows":[]}"#.into());
    let none = c.search_perfumes(&by_year(2020), 1).await.unwrap();
    assert_eq!((none.items.len(), none.total_items, none.total_pages), (0, Some(0), Some(0)));
}

#[tokio::test]
async fn a_listing_without_its_count_or_a_refusal_in_json_is_left_to_the_web_search() {
    let (c, site) = logged_in().await;
    let no_count = PERFUMES_TEXT.replace("\"matches\": 151,", "");
    assert_ne!(no_count, PERFUMES_TEXT);
    for answer in [no_count, PERFUMES_TEXT.replace("\"matches\": 151", "\"matches\": null"), r#"{"error":"Not available"}"#.to_string()] {
        *site.perfumes.lock().unwrap() = Some(answer.clone());
        // The fake has no web search: what matters is that it is asked, where a count read
        // as zero would have ended the listing on "0 matches".
        let _ = c.search_perfumes(&by_year(2020), 1).await;
        let asked = site.take();
        assert_eq!(asked.len(), 2, "{asked:?}");
        assert!(asked[0].starts_with("/app/v1/perfumes?"), "{asked:?}");
        assert!(asked[1].starts_with("/s_perfumes_x.php?"), "{answer}: {asked:?}");
    }
    // An answer of another shape that is not a refusal is reported.
    *site.perfumes.lock().unwrap() = Some(r#"{"matches":5}"#.into());
    assert_eq!(c.search_perfumes(&by_year(2020), 1).await.unwrap_err(), ClientError::ParserOutdated("/app/v1/perfumes: no rows".into()));
    assert!(c.has_app_session(), "none of this is a refusal of the session");
}

// ---------------------------------------------------------------------------
// The record of a perfume
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_perfume_record_is_read_by_number_with_the_parts_of_the_web_record() {
    let (c, site) = logged_in().await;
    let p = c.get_perfume("9438").await.unwrap();
    assert_eq!(site.take(), ["/app/v1/perfume_details/9438"], "one call, no web page");

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

    assert_eq!((p.reviews_count, p.statements_count), (Some(632), Some(1721)));
    assert!(p.description.as_deref().unwrap().starts_with("A popular perfume by Creed"));
    // The address is built from the base address of the client, like every other one of the record.
    assert_eq!(p.url, format!("{}/Perfumes/Creed/aventus", c.base_url()));
    assert!(p.image_url.is_some());
    // The rank label of the web page is not in the answer of the interface: it stays
    // absent, and the record of this perfume has no German-only fact to leave out.
    assert_eq!((p.ranking, p.facts), (None, None));
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[tokio::test]
async fn the_classification_is_on_the_scale_and_in_the_order_of_the_web_record() {
    let (c, _site) = logged_in().await;
    let p = c.get_perfume("9438").await.unwrap();

    // The groups in the order of the web record (the interface sends Season before
    // Occasion), each with its own votes.
    assert_eq!(
        groups(&p),
        [
            ("Type", Some(6884), "Fresh"),
            ("Occasion", Some(7712), "Daily"),
            ("Season", Some(5733), "Spring"),
            ("Audience & Style", Some(3848), "Masculine")
        ]
    );
    // Type: the shares of the interface count the types it does not list (they add up to
    // 84.3); the web chart gives them out of the listed ones, strongest first.
    assert_eq!(
        shares(&p, "Type"),
        [("Fresh", 22.8), ("Fruity", 21.0), ("Citrus", 17.0), ("Woody", 13.9), ("Smoky", 12.1), ("Spicy", 8.0), ("Sweet", 5.2)]
    );
    assert_eq!(shares(&p, "Season"), [("Spring", 32.0), ("Summer", 30.6), ("Fall", 23.8), ("Winter", 13.7)]);
    for group in ["Type", "Occasion", "Season"] {
        let sum: f32 = shares(&p, group).iter().map(|(_, percent)| percent).sum();
        assert!((sum - 100.0).abs() < 0.25, "{group}: {sum}");
    }
    // Audience & Style: two pairs that each add up to 100, as on the web chart. The
    // interface gives one share over the four poles (48, 27, 22) and leaves Feminine out.
    assert_eq!(shares(&p, "Audience & Style"), [("Masculine", 93.6), ("Feminine", 6.4), ("Modern", 55.7), ("Classic", 44.3)]);
    // The accords of the web page: the five strongest types, not the whole chart.
    assert_eq!(p.accords, ["Fresh", "Fruity", "Citrus", "Woody", "Smoky"]);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[tokio::test]
async fn a_chart_keeps_its_name_and_its_votes_whatever_the_other_charts_are() {
    let (c, site) = logged_in().await;
    // Not recorded: the recorded record with its Season chart missing, then with its charts
    // sent as objects keyed by position, the other shape the interface uses for a list.
    site.records.lock().unwrap().insert(1, record(|r| r["c"][1] = Value::Null));
    let p = c.get_perfume("1").await.unwrap();
    assert_eq!(groups(&p), [("Type", Some(6884), "Fresh"), ("Occasion", Some(7712), "Daily"), ("Audience & Style", Some(3848), "Masculine")]);
    assert_eq!(p.accords[0], "Fresh");

    site.records.lock().unwrap().insert(
        2,
        record(|r| {
            let keyed = |list: &Value| Value::Object(list.as_array().unwrap().iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect());
            let charts: Vec<Value> = r["c"].as_array().unwrap().iter().map(keyed).collect();
            r["c"] = keyed(&Value::Array(charts));
        }),
    );
    let keyed = c.get_perfume("2").await.unwrap();
    assert_eq!(keyed.classification, c.get_perfume("9438").await.unwrap().classification);
}

#[tokio::test]
async fn a_pair_of_audience_and_style_that_cannot_be_rebuilt_is_left_out_and_told() {
    let (c, site) = logged_in().await;
    let poles = |keep: &[&str]| {
        let keep: Vec<String> = keep.iter().map(|k| k.to_string()).collect();
        record(move |r| r["c"][3].as_array_mut().unwrap().retain(|s| keep.iter().any(|k| s["n"] == k.as_str())))
    };
    // Not recorded: the recorded record with fewer poles. With Modern alone of its pair,
    // and Feminine missing too, what is left of 100 cannot be shared between the two.
    site.records.lock().unwrap().insert(1, poles(&["Masculine", "Modern"]));
    let p = c.get_perfume("1").await.unwrap();
    assert!(p.classification.iter().all(|g| g.group != "Audience & Style"), "no raw share is shown: {:?}", p.classification);
    assert_eq!(p.warnings.len(), 2, "{:?}", p.warnings);
    assert!(p.warnings[0].contains("Masculine/Feminine") && p.warnings[1].contains("Modern/Classic"), "{:?}", p.warnings);

    // A whole pair missing: the other one is still a pair of its own.
    site.records.lock().unwrap().insert(2, poles(&["Classic", "Modern"]));
    let p = c.get_perfume("2").await.unwrap();
    assert_eq!(shares(&p, "Audience & Style"), [("Modern", 55.7), ("Classic", 44.3)]);
    assert_eq!(p.warnings.len(), 1, "{:?}", p.warnings);
    assert!(p.warnings[0].starts_with("classification: the Masculine/Feminine shares"), "{:?}", p.warnings);

    // A perfume nobody classified has no chart, and that is not worth a warning.
    site.records.lock().unwrap().insert(3, record(|r| r["c"] = json!([[], [], [], []])));
    let p = c.get_perfume("3").await.unwrap();
    assert!(p.classification.is_empty() && p.accords.is_empty() && p.warnings.is_empty(), "{:?}", p.warnings);
}

#[tokio::test]
async fn facts_and_description_are_plain_text_and_never_the_german_ones() {
    let (c, site) = logged_in().await;
    // Not recorded: the recorded record has no facts. The same record with English facts
    // and a description written with the markup of the site, and German facts beside them.
    site.records.lock().unwrap().insert(
        1,
        record(|r| {
            r["p"]["info_en"] = "First fact.<br />\r\nSecond fact, by <a href=\"/Perfumers/x\">someone</a> &amp; co.".into();
            r["p"]["info"] = "Deutsche Fakten.".into();
            r["p"]["desc"] = "A perfume by <b>Creed</b> &amp; friends.".into();
        }),
    );
    let p = c.get_perfume("1").await.unwrap();
    assert_eq!(p.facts.as_deref(), Some("First fact.\nSecond fact, by someone & co."));
    assert_eq!(p.description.as_deref(), Some("A perfume by Creed & friends."));

    // German facts alone: the English page shows none, and neither does the record.
    site.records.lock().unwrap().insert(2, record(|r| r["p"]["info"] = "Deutsche Fakten.".into()));
    assert_eq!(c.get_perfume("2").await.unwrap().facts, None);
}

// ---------------------------------------------------------------------------
// Which perfume is meant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_perfume_named_by_its_id_is_found_through_its_brand_then_remembered() {
    let (c, site) = logged_in().await;
    // Never met in a listing: the brand is looked up, then the perfume among its own.
    let p = c.get_perfume("/Perfumes/Creed/absolu-aventus").await.unwrap();
    assert_eq!((p.id.as_str(), p.p_id), ("Creed/absolu-aventus", Some(225233)));
    assert_eq!(
        site.take(),
        [
            "/app/v1/search?filter=Creed",
            "/app/v1/perfumes?page=1&per_page=20&filter=absolu+aventus&b_id=253&g_f=1&g_m=1&g_u=1",
            "/app/v1/perfume_details/225233"
        ]
    );
    // Met once, known from then on, whatever the letter case of the reference.
    c.get_perfume("creed/Absolu-Aventus").await.unwrap();
    assert_eq!(site.take(), ["/app/v1/perfume_details/225233"], "only the record itself is asked again");
    assert!(matches!(c.get_perfume("not a reference").await.unwrap_err(), ClientError::InvalidInput(_)));
    assert!(site.take().is_empty());
}

#[tokio::test]
async fn a_perfume_the_lookup_of_the_interface_misses_is_read_from_the_web_site() {
    let (c, site) = logged_in().await;
    // The search of the interface does not list the brand under this slug: that says
    // nothing of Parfumo. The web page is read, as it is without a session.
    let p = c.get_perfume(SAUVAGE).await.unwrap();
    assert_eq!(
        site.take(),
        ["/app/v1/search?filter=Dior", "/Perfumes/Dior/Sauvage_Eau_de_Toilette", "/api/alive", "/action/perfume/get_classification_chart.php"]
    );
    assert_eq!((p.id.as_str(), p.name.as_str(), p.concentration.as_deref()), (SAUVAGE, "Sauvage", Some("Eau de Toilette")));
    assert_eq!(p.ranking.as_deref(), Some("Ranked 30 in Men's Perfume"), "the record of the web page");

    // The brand is found and the perfume is not on the page of its perfumes that was read:
    // the web site has the last word, here that there is no such page.
    let err = c.get_perfume("Creed/no-such-perfume").await.unwrap_err();
    assert_eq!(
        site.take(),
        [
            "/app/v1/search?filter=Creed",
            "/app/v1/perfumes?page=1&per_page=20&filter=no+such+perfume&b_id=253&g_f=1&g_m=1&g_u=1",
            "/Perfumes/Creed/no-such-perfume"
        ]
    );
    assert_eq!(err, ClientError::NotFound("/Perfumes/Creed/no-such-perfume".into()));
    assert!(!site.token_on_web.load(Ordering::SeqCst), "the session token is for the application interface only");
    assert!(c.has_app_session(), "a miss is not a refusal of the session");
}

#[tokio::test]
async fn a_number_the_interface_does_not_have_is_not_found_and_a_remembered_one_goes_to_the_web_site() {
    let (c, site) = logged_in().await;
    // A bare number has no address on the web site: the interface alone can tell, by a 404
    // or by a refusal in JSON under status 200, and neither is a change of the interface.
    for number in [UNKNOWN, REFUSED] {
        let err = c.get_perfume(&number.to_string()).await.unwrap_err();
        assert_eq!(err, ClientError::NotFound(format!("/perfume_details/{number}")));
        assert_eq!(site.take(), [format!("/app/v1/perfume_details/{number}")], "and no web page is asked");
    }

    // A number remembered for an id, which the interface no longer has (a perfume merged or
    // removed since): the web page of the id answers, and the number is forgotten.
    let gone = PERFUMES_BRAND.replace("\"p_id\": \"225233\"", &format!("\"p_id\": \"{UNKNOWN}\""));
    assert_ne!(gone, PERFUMES_BRAND);
    *site.perfumes.lock().unwrap() = Some(gone);
    c.search_perfumes(&by_year(2023), 1).await.unwrap();
    site.take();
    let err = c.get_perfume("Creed/absolu-aventus").await.unwrap_err();
    assert_eq!(site.take(), [format!("/app/v1/perfume_details/{UNKNOWN}"), "/Perfumes/Creed/absolu-aventus".to_string()]);
    assert_eq!(err, ClientError::NotFound("/Perfumes/Creed/absolu-aventus".into()));
    *site.perfumes.lock().unwrap() = None;
    let _ = c.get_perfume("Creed/absolu-aventus").await;
    assert_eq!(site.take()[0], "/app/v1/search?filter=Creed", "looked up afresh");
}

// ---------------------------------------------------------------------------
// Reviews and statements: always the web site
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reviews_and_statements_are_read_from_the_web_site_even_with_a_session() {
    let (c, site) = logged_in().await;
    let web = |fragment: &str| ["/Perfumes/Dior/Sauvage_Eau_de_Toilette".to_string(), format!("/action/perfume/{fragment}")];

    // The default order and the two the interface knows: none of them goes to it.
    for sort in [ReviewSort::Relevance, ReviewSort::Newest, ReviewSort::Oldest] {
        let reviews = c.get_perfume_reviews(SAUVAGE, sort, 0, 10).await.unwrap();
        assert_eq!(site.take(), web("get_reviews.php"), "{sort:?}: nothing is asked of the interface");
        // What the interface does not give: every scale the member rated, the day of the
        // review, and how many reviews there are.
        let first = &reviews.items[0];
        assert_eq!((first.id, first.author.as_str()), (721243, "member315"));
        assert_eq!((first.ratings.scent, first.ratings.durability, first.ratings.sillage), (Some(7.0), Some(7.0), Some(7.0)));
        assert_eq!(first.date.as_deref(), Some("2026-10-25"));
        assert_eq!((reviews.offset, reviews.total), (0, Some(378)));
    }

    let statements = c.get_perfume_statements(SAUVAGE, ReviewSort::Relevance, 0, 50).await.unwrap();
    assert_eq!(site.take(), web("get_statements.php"));
    assert_eq!((statements.items[0].id, statements.items[0].ratings.durability), (2800120, Some(6.0)));
    assert_eq!(statements.total, Some(1322));

    assert!(!site.token_on_web.load(Ordering::SeqCst), "the session token is for the application interface only");
    assert!(c.has_app_session());
}
