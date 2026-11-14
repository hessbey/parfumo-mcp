//! The member's own collection through the application interface, against answers recorded
//! on the real site with the test account (`tests/fixtures/app_shelf_*.json`). A local fake
//! serves them, and the ShelfControl page the client falls back to, and records what is asked.

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
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use parfumo_client::{ClientConfig, ClientError, CustomColumn, GlobalLimiter, ParfumoClient};
use serde_json::{Value, json};

/// `GET /shelfcontrol?list=have`, with two entries in the list and without any.
const SHEET: &str = include_str!("fixtures/app_shelf_have.json");
const EMPTY_SHEET: &str = include_str!("fixtures/app_shelf_empty.json");
/// `GET /wardrobe?type=have`.
const WARDROBE: &str = include_str!("fixtures/app_shelf_wardrobe_have.json");
/// `GET /users/{nick}/collection?type=have`.
const MEMBER: &str = include_str!("fixtures/app_shelf_member_have.json");
/// The ShelfControl page of the web site: one entry, a custom column, a custom list.
const WEB: &str = include_str!("fixtures/auth_shelfcontrol.html");

/// The custom list of the web page. The test account had none when the interface was
/// recorded: the fake names it as the application reads one, by its number.
const CUSTOM: u64 = 922_018;

// Only "I have" was recorded. What the fake answers for another list, and each twist
// below, stands for what the client must withstand, not for what the site is known to do.
/// What the fake changes in its answers.
#[derive(Clone, Copy, PartialEq)]
enum Twist {
    /// The member has a custom column, and a value in it.
    Column,
    /// The member has a custom column, and no value in it.
    EmptyColumn,
    /// A value of a custom column on a sheet that has no column.
    StrayValue,
    /// The sheet of another list than the one asked.
    OtherSheet,
    /// A custom list whose `type` is not its number.
    OddType,
    /// A list without a count.
    NoCount,
    /// A row without a title.
    NoTitle,
    /// Perfumes that have a concentration: one titled without it, one with it.
    Concentration,
    /// The details of another list than the one asked.
    OtherDetails,
    /// Details that do not say whether the list is hidden.
    NoHidden,
    /// The profile of the member shows one perfume less than the sheet.
    Unshown,
    /// The profile answers for another list than the one asked.
    OtherShown,
    /// The profile of the member is not served.
    NoProfile,
    /// The default list bears another name, as in another language.
    Renamed,
    /// A nickname that cannot be written as it is in an address.
    Nick,
}

#[derive(Default)]
struct Site {
    /// Path and query of every request, in order; after its method when it is not a `GET`.
    asked: Mutex<Vec<String>>,
    /// Whether the token is refused from now on.
    refusing: AtomicBool,
    twist: Mutex<Option<Twist>>,
}

/// Number of a list from its `type`.
fn number_of(list: &str) -> Option<u64> {
    let fixed = ["have", "had", "want", "memo", "tested", "filling", "mini"];
    fixed.iter().position(|slug| *slug == list).map(|at| at as u64 + 1).or_else(|| list.parse().ok().filter(|nr| *nr == CUSTOM))
}

/// The sheet of a list: the recorded one, as the site gives it for the list asked.
fn sheet(nr: u64, twist: Option<Twist>) -> Value {
    // "I had" is empty: the answer recorded without entries.
    let mut answer: Value = serde_json::from_str(if nr == 2 { EMPTY_SHEET } else { SHEET }).unwrap();
    answer["list"] = nr.into();
    let custom_type = if twist == Some(Twist::OddType) { format!("custom_{CUSTOM}") } else { CUSTOM.to_string() };
    answer["lists"].as_array_mut().unwrap().push(json!({"id": CUSTOM, "type": custom_type, "name": "TestList", "icon": "", "count": 2}));
    if nr == 3 {
        // The recorded rows have none of the fields a member fills: here as the web page records them.
        let row = &mut answer["rows"][0];
        for (key, value) in [("qty", json!(2)), ("bottle", json!("100")), ("ml", json!(60)), ("paid", json!(85)), ("cur", json!("USD"))] {
            row[key] = value;
        }
        row["memo"] = json!(" a note ");
        row["statement"] = json!(12);
        // And values at the edges of what a row may hold.
        let row = &mut answer["rows"][1];
        for (key, value) in [("ratings", json!([100, 5, 0, null, "75"])), ("bottle", json!("0")), ("year", json!(0)), ("gender", json!("weiblich"))] {
            row[key] = value;
        }
        row["review"] = json!(7);
    }
    match twist {
        Some(Twist::Column) => {
            answer["columns"] = json!([{"id": "178", "name": "TestCol", "type": "text"}]);
            answer["rows"][0]["custom"] = json!({"178": "a value"});
        }
        Some(Twist::EmptyColumn) => answer["columns"] = json!([{"id": "178", "name": "TestCol", "type": "text"}]),
        Some(Twist::StrayValue) => answer["rows"][0]["custom"] = json!({"178": "a value"}),
        Some(Twist::OtherSheet) => answer["list"] = 2.into(),
        Some(Twist::NoCount) => {
            answer["lists"][3].as_object_mut().unwrap().remove("count");
        }
        Some(Twist::NoTitle) => {
            answer["rows"][1].as_object_mut().unwrap().remove("title");
        }
        Some(Twist::Concentration) => {
            answer["rows"][0]["conc"] = json!("Eau de Parfum");
            answer["rows"][1]["conc"] = json!("Eau de Toilette");
            answer["rows"][1]["title"] = json!("Stronger With You Intensely (Eau de Toilette)");
        }
        Some(Twist::Renamed) => answer["lists"][0]["name"] = json!("Je possède"),
        _ => {}
    }
    answer
}

/// The same list as the application shows it: the details of the list, the nickname.
fn wardrobe(nr: u64, twist: Option<Twist>) -> Value {
    let mut answer: Value = serde_json::from_str(WARDROBE).unwrap();
    answer["list_details"]["id"] = nr.into();
    match nr {
        3 => answer["list_details"]["hidden"] = json!("Yes"),
        CUSTOM => answer["list_details"]["notes"] = json!("A description"),
        _ => {}
    }
    match twist {
        Some(Twist::OtherDetails) => answer["list_details"]["id"] = 2.into(),
        Some(Twist::NoHidden) => answer["list_details"]["hidden"] = json!(""),
        Some(Twist::Nick) => answer["nick_name"] = json!("test user+é"),
        _ => {}
    }
    answer
}

/// The list as the profile of the member shows it.
fn member(list: &str, twist: Option<Twist>) -> Value {
    let mut answer: Value = serde_json::from_str(MEMBER).unwrap();
    answer["type"] = list.into();
    match twist {
        Some(Twist::Unshown) => {
            answer["perfumes"].as_array_mut().unwrap().pop();
        }
        Some(Twist::OtherShown) => answer["type"] = json!("had"),
        _ => {}
    }
    answer
}

async fn handler(State(site): State<Arc<Site>>, method: Method, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    // A read only asks: anything else shows in what was asked, and fails the test that looks at it.
    site.asked.lock().unwrap().push(if method == Method::GET { asked } else { format!("{method} {asked}") });
    let json = |answer: Value| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], answer.to_string()).into_response();
    // The web site: the page of the list asked, and the default one for a list it does not know.
    if let Some(rest) = uri.path().strip_prefix("/account/shelfcontrol") {
        let nr = number_of(rest.trim_start_matches('/')).unwrap_or(1);
        let page = WEB.replace("data-list=\"1\"", &format!("data-list=\"{nr}\""));
        return (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], page).into_response();
    }
    if uri.path() == "/app/v1/auth/login" {
        return json(json!({"status": "success", "auth_token": "tok"}));
    }
    if site.refusing.load(Ordering::SeqCst) || headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    let query: HashMap<String, String> = url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).into_owned().collect();
    let param = |key: &str| query.get(key).map(String::as_str).unwrap_or_default();
    let twist = *site.twist.lock().unwrap();
    let nick = if twist == Some(Twist::Nick) { "test%20user%2B%C3%A9" } else { "testuser" };
    match uri.path() {
        // Like a site that would answer with the default list for one it does not know.
        "/app/v1/shelfcontrol" => json(sheet(number_of(param("list")).unwrap_or(1), twist)),
        "/app/v1/wardrobe" => json(wardrobe(number_of(param("type")).unwrap_or(1), twist)),
        path if path == format!("/app/v1/users/{nick}/collection") && twist != Some(Twist::NoProfile) => json(member(param("type"), twist)),
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

/// What was asked since the last look.
fn asked(site: &Site) -> Vec<String> {
    std::mem::take(&mut *site.asked.lock().unwrap())
}

/// The three calls of a list read for the first time.
fn three_calls(list: &str) -> Vec<String> {
    vec![
        format!("/app/v1/shelfcontrol?list={list}"),
        format!("/app/v1/wardrobe?type={list}"),
        format!("/app/v1/users/testuser/collection?type={list}"),
    ]
}

fn texts(items: &[String]) -> Vec<&str> {
    items.iter().map(String::as_str).collect()
}

#[tokio::test]
async fn a_list_is_read_in_three_calls_and_in_two_once_its_perfumes_are_known() {
    let (c, site) = logged_in().await;
    let shelf = c.shelf("have").await.unwrap();
    assert_eq!(asked(&site), three_calls("have"), "the sheet, the details of the list, the addresses; no web page");

    let list = &shelf.list;
    assert_eq!((list.id, list.slug.as_str(), list.name.as_str(), list.count, list.custom), (1, "have", "I have", 2, false));
    assert_eq!((list.hidden, list.notes.as_deref()), (false, None), "told by the details of the list");
    assert_eq!(list.url, format!("{}/account/shelfcontrol/have", c.base_url()));
    assert_eq!(shelf.lists.len(), 8);
    assert_eq!(&shelf.lists[0], list, "the list read is one of the lists");
    let lists: Vec<_> = shelf.lists.iter().map(|l| (l.id, l.slug.as_str(), l.name.as_str(), l.count, l.custom)).collect();
    assert_eq!(
        lists,
        [
            (1, "have", "I have", 2, false),
            (2, "had", "I had", 0, false),
            (3, "want", "Wish List", 1, false),
            (4, "memo", "Watch List", 0, false),
            (5, "tested", "Tested", 0, false),
            (6, "filling", "Decants", 0, false),
            (7, "mini", "Miniatures", 0, false),
            (CUSTOM, "922018", "TestList", 2, true),
        ]
    );
    assert_eq!(shelf.lists[7].url, format!("{}/account/shelfcontrol/922018", c.base_url()));
    // Whether the other lists are hidden, and their description, is not told: nothing is made up.
    assert!(shelf.lists[1..].iter().all(|l| !l.hidden && l.notes.is_none()));
    assert!(shelf.columns.is_empty());

    assert_eq!(shelf.items.len(), 2);
    let i = &shelf.items[0];
    assert_eq!((i.entry_id, i.p_id, i.perfume_id.as_str()), (119640142, 229262, "Rasasi/hawas-ice-ice"));
    assert_eq!(i.url, format!("{}/Perfumes/Rasasi/hawas-ice-ice", c.base_url()));
    assert_eq!((i.name.as_str(), i.brand.as_deref(), i.country.as_deref()), ("Hawas Ice / هوس Ice", Some("Rasasi"), Some("United Arab Emirates")));
    assert_eq!((i.year, i.gender.as_deref(), i.concentration.as_deref()), (Some(2023), Some("men"), None));
    assert!(i.my_ratings.is_empty(), "not rated: no rating, not zero");
    assert_eq!((i.community_rating, i.community_votes), (Some(7.8), Some(1078)));
    assert_eq!((i.quantity, i.bottle_size.as_deref(), i.remaining_ml, i.price_paid, i.currency.as_deref()), (Some(1), None, None, None, Some("EUR")));
    assert_eq!((i.private_note.as_deref(), i.added.as_deref()), (None, Some("2026-11-13")));
    assert_eq!((i.wear_count, i.last_worn.as_deref()), (0, None));
    assert!(!i.has_statement && !i.has_review && i.custom.is_empty() && i.perfumers.is_empty());
    assert_eq!(texts(&i.accords), ["Fresh", "Sweet", "Fruity"]);
    assert_eq!((texts(&i.seasons), texts(&i.occasions)), (vec!["Summer", "Spring"], vec!["Daily", "Leisure"]));
    assert_eq!((i.notes.len(), i.notes[0].as_str(), i.notes[12].as_str()), (13, "Frosted apple", "Musk"));

    let i = &shelf.items[1];
    assert_eq!((i.entry_id, i.p_id), (119640117, 120220));
    assert_eq!(i.perfume_id, "Giorgio_Armani/emporio-armani-stronger-with-you-intensely-1");
    assert_eq!(i.url, format!("{}/Perfumes/Giorgio_Armani/emporio-armani-stronger-with-you-intensely-1", c.base_url()));
    assert_eq!(
        (i.name.as_str(), i.brand.as_deref(), i.country.as_deref()),
        ("Emporio Armani - Stronger With You Intensely", Some("Giorgio Armani"), Some("Italy"))
    );
    let r = &i.my_ratings;
    assert_eq!((r.scent, r.durability, r.sillage, r.bottle, r.pricing), (Some(8.0), Some(9.0), Some(7.0), Some(6.0), Some(9.0)), "out of ten");
    assert_eq!((i.community_rating, i.community_votes), (Some(8.3), Some(5448)));
    assert_eq!((i.wear_count, i.last_worn.as_deref(), i.added.as_deref()), (1, Some("2026-11-13"), Some("2026-11-13")));
    assert_eq!(texts(&i.perfumers), ["Cécile Matton", "Mane"]);
    assert_eq!(texts(&i.accords), ["Sweet", "Gourmand", "Spicy"]);
    assert_eq!((texts(&i.seasons), texts(&i.occasions)), (vec!["Winter", "Fall"], vec!["Evening", "Night Out"]));
    assert_eq!(texts(&i.notes), ["Pink pepper", "Chestnut", "Lavender", "Sage", "Vanilla", "Amberwood"]);

    // The addresses are remembered: the list is read again without asking for them.
    let again = c.shelf("have").await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/shelfcontrol?list=have", "/app/v1/wardrobe?type=have"]);
    assert_eq!(again, shelf);
    // And they serve the other reads: a perfume of the list is known by its id, without a search.
    // (The fake has no record of a perfume: only what is asked matters here.)
    assert!(matches!(c.get_perfume("rasasi/Hawas-Ice-Ice").await, Err(ClientError::NotFound(_))));
    assert_eq!(asked(&site), ["/app/v1/perfume_details/229262"]);
}

#[tokio::test]
async fn a_list_is_named_by_slug_alias_number_or_name() {
    let (c, site) = logged_in().await;
    for reference in ["I_have", " collection ", "1", "HAVE"] {
        assert_eq!(c.shelf(reference).await.unwrap().list.id, 1, "{reference}");
        assert_eq!(asked(&site)[0], "/app/v1/shelfcontrol?list=have", "{reference}");
    }
    for reference in ["Wish List", "3", "wishlist"] {
        assert_eq!(c.shelf(reference).await.unwrap().list.slug, "want", "{reference}");
        assert_eq!(asked(&site)[..2], ["/app/v1/shelfcontrol?list=want", "/app/v1/wardrobe?type=want"], "{reference}");
    }

    // A custom list by its number: asked under that number, as the application does.
    let custom = c.shelf("922018").await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/shelfcontrol?list=922018", "/app/v1/wardrobe?type=922018"]);
    assert_eq!((custom.list.id, custom.list.slug.as_str(), custom.list.custom, custom.list.count), (CUSTOM, "922018", true, 2));
    assert_eq!((custom.list.hidden, custom.list.notes.as_deref()), (false, Some("A description")));
    assert_eq!(custom.lists[7], custom.list);
    assert_eq!(custom.items.len(), 2);

    // By its name: the default sheet names the lists, then its own sheet is read.
    let named = c.shelf(" testlist ").await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/shelfcontrol?list=have", "/app/v1/shelfcontrol?list=922018", "/app/v1/wardrobe?type=922018"]);
    assert_eq!(named, custom);
}

#[tokio::test]
async fn a_name_that_designates_the_default_list_needs_no_second_sheet() {
    let (c, site) = logged_in().await;
    *site.twist.lock().unwrap() = Some(Twist::Renamed);
    let shelf = c.shelf("je possède").await.unwrap();
    assert_eq!(asked(&site), three_calls("have"));
    assert_eq!((shelf.list.id, shelf.list.name.as_str(), shelf.items.len()), (1, "Je possède", 2));
}

#[tokio::test]
async fn what_the_member_filled_and_a_hidden_list_are_read() {
    let (c, site) = logged_in().await;
    let shelf = c.shelf("want").await.unwrap();
    assert_eq!(asked(&site), three_calls("want"));
    assert_eq!((shelf.list.id, shelf.list.hidden, shelf.list.notes.as_deref()), (3, true, None));
    assert!(shelf.lists[2].hidden && !shelf.lists[0].hidden);
    let i = &shelf.items[0];
    assert_eq!(
        (i.quantity, i.bottle_size.as_deref(), i.remaining_ml, i.price_paid, i.currency.as_deref()),
        (Some(2), Some("100"), Some(60.0), Some(85.0), Some("USD"))
    );
    assert_eq!((i.private_note.as_deref(), i.has_statement, i.has_review), (Some("a note"), true, false));

    let i = &shelf.items[1];
    let r = &i.my_ratings;
    assert_eq!((r.scent, r.durability, r.sillage, r.bottle, r.pricing), (Some(10.0), Some(0.5), Some(0.0), None, Some(7.5)), "0 is a rating");
    assert_eq!((i.bottle_size.as_deref(), i.year, i.gender.as_deref()), (None, None, Some("women")), "0 is no size and no year");
    assert_eq!((i.has_statement, i.has_review), (false, true));
}

#[tokio::test]
async fn a_name_carries_its_concentration_once() {
    let (c, site) = logged_in().await;
    *site.twist.lock().unwrap() = Some(Twist::Concentration);
    let shelf = c.shelf("have").await.unwrap();
    assert_eq!(asked(&site), three_calls("have"));
    // As the web page titles a perfume, whether the interface puts the concentration in the title or not.
    let named: Vec<_> = shelf.items.iter().map(|i| (i.name.as_str(), i.concentration.as_deref())).collect();
    assert_eq!(
        named,
        [("Hawas Ice / هوس Ice (Eau de Parfum)", Some("Eau de Parfum")), ("Stronger With You Intensely (Eau de Toilette)", Some("Eau de Toilette"))]
    );
}

#[tokio::test]
async fn an_empty_list_is_read_in_two_calls() {
    let (c, site) = logged_in().await;
    let shelf = c.shelf("had").await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/shelfcontrol?list=had", "/app/v1/wardrobe?type=had"], "no address to ask for");
    assert_eq!((shelf.list.id, shelf.list.slug.as_str(), shelf.list.count), (2, "had", 0));
    assert!(shelf.items.is_empty() && shelf.columns.is_empty());
    assert_eq!(shelf.lists.len(), 8);
}

#[tokio::test]
async fn a_nickname_is_encoded_in_the_address() {
    let (c, site) = logged_in().await;
    *site.twist.lock().unwrap() = Some(Twist::Nick);
    assert_eq!(c.shelf("have").await.unwrap().items.len(), 2);
    assert_eq!(asked(&site)[2], "/app/v1/users/test%20user%2B%C3%A9/collection?type=have");
}

#[tokio::test]
async fn what_is_not_certain_is_left_to_the_web_site() {
    let sheet = "/app/v1/shelfcontrol?list=have".to_string();
    let details = "/app/v1/wardrobe?type=have".to_string();
    let cases = [
        (Twist::Column, vec![sheet.clone()]),
        (Twist::EmptyColumn, vec![sheet.clone()]),
        (Twist::StrayValue, vec![sheet.clone()]),
        (Twist::OtherSheet, vec![sheet.clone()]),
        (Twist::OddType, vec![sheet.clone()]),
        (Twist::NoCount, vec![sheet.clone()]),
        (Twist::NoTitle, vec![sheet.clone()]),
        (Twist::OtherDetails, vec![sheet.clone(), details.clone()]),
        (Twist::NoHidden, vec![sheet.clone(), details.clone()]),
        (Twist::Unshown, three_calls("have")),
        (Twist::OtherShown, three_calls("have")),
        (Twist::NoProfile, three_calls("have")),
    ];
    for (case, (twist, mut calls)) in cases.into_iter().enumerate() {
        // A client that met no perfume yet: the addresses are to be asked for.
        let (c, site) = logged_in().await;
        *site.twist.lock().unwrap() = Some(twist);
        let shelf = c.shelf("have").await.unwrap();
        calls.push("/account/shelfcontrol/have".into());
        assert_eq!(asked(&site), calls, "case {case}");
        // The page of the web site, whole: its one entry, its custom column.
        assert_eq!((shelf.items.len(), shelf.items[0].perfume_id.as_str()), (1, "Dior/Sauvage_Eau_de_Toilette"), "case {case}");
        assert_eq!(shelf.columns, [CustomColumn { id: 178, name: "TestCol".into() }], "case {case}");
    }

    let (c, site) = logged_in().await;
    // A number no list has: the interface answers with another list, the web site says it is unknown.
    assert!(matches!(c.shelf("555555").await.unwrap_err(), ClientError::NotFound(_)));
    assert_eq!(asked(&site), ["/app/v1/shelfcontrol?list=555555", "/account/shelfcontrol/555555"]);
    // A name no list bears.
    assert!(matches!(c.shelf("my imaginary list").await.unwrap_err(), ClientError::NotFound(_)));
    assert_eq!(asked(&site), ["/app/v1/shelfcontrol?list=have", "/account/shelfcontrol"]);
    // Not the number of a list: the interface is not asked.
    assert!(matches!(c.shelf("0").await.unwrap_err(), ClientError::NotFound(_)));
    assert_eq!(asked(&site), ["/account/shelfcontrol/0"]);
}

#[tokio::test]
async fn the_lists_with_what_is_hidden_stay_on_the_web_page() {
    let (c, site) = logged_in().await;
    // Hidden or not, and the description, of every list: one call per list through the interface.
    let shelf = c.shelf_default().await.unwrap();
    assert_eq!(asked(&site), ["/account/shelfcontrol"]);
    assert_eq!((shelf.list.id, shelf.lists.len(), shelf.columns.len()), (1, 8, 1));

    // Without a session of the application interface, nothing is asked of it.
    c.forget_app_session();
    assert_eq!(c.shelf("have").await.unwrap().items.len(), 1);
    assert_eq!(asked(&site), ["/account/shelfcontrol/have"]);
}

#[tokio::test]
async fn a_refused_token_is_not_sent_again() {
    let (c, site) = logged_in().await;
    site.refusing.store(true, Ordering::SeqCst);
    assert_eq!(c.shelf("have").await.unwrap_err(), ClientError::LoginRequired);
    assert_eq!(asked(&site), ["/app/v1/shelfcontrol?list=have"]);
    // The token is forgotten: the next read is the one of the web site.
    assert!(!c.has_app_session());
    assert_eq!(c.shelf("have").await.unwrap().list.id, 1);
    assert_eq!(asked(&site), ["/account/shelfcontrol/have"]);
}
