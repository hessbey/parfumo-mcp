//! Member profiles and their collection lists through the application interface, against
//! answers recorded on the real site (`tests/fixtures/app_members_*.json`, scrubbed: the
//! nicknames, the member numbers, the photos, the countries and the dates are placeholders).
//! A local fake serves them, and the web pages the client falls back to, and records what
//! is asked.

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
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient, TextKind};

/// `GET /users/<own nick>`, with a perfume worn, two perfumes in "I have" and one in the wish list.
const PROFILE: &str = include_str!("fixtures/app_members_profile.json");
/// `GET /users/<another member>`: nothing worn, every list empty.
const PROFILE_OTHER: &str = include_str!("fixtures/app_members_profile_other.json");
/// `GET /users/<own nick>/collection?type=have`: the two perfumes.
const COLLECTION: &str = include_str!("fixtures/app_members_collection.json");
/// `GET /users/<another member>/collection?type=have`, as recorded: an empty list.
const COLLECTION_EMPTY: &str = r#"{"type":"have","perfumes":[]}"#;
/// Not recorded, written in the recorded shape: a row with a concentration, named as the rows of a note are.
const COLLECTION_EDP: &str = r#"{"type":"have","perfumes":[{"id":102281,"p_name":"Sauvage","p_name_url":"Sauvage_Eau_de_Parfum","concentration":"Eau de Parfum","gender":"maennlich","release_year":2018,"b_id":32,"b_name":"Dior","b_name_url":"Dior","url_image_240":"https://media.parfumo.com/perfumes/placeholder.jpg"}]}"#;
/// Not recorded: what the application reads of a custom list.
const CUSTOM_LISTS: &str = r#""lists": [{"id": 4001, "name": "Summer", "count": 3, "icon": 5}]"#;

/// The web pages the client falls back to: one member, whatever the nickname.
const WEB_PROFILE: &str = include_str!("fixtures/user_profile.html");
const WEB_COLLECTION: &str = include_str!("fixtures/user_collection_list.html");
const WEB_WARDROBE: &str = include_str!("fixtures/frag_user_wardrobe.html");
const WEB_REVIEWS: &str = include_str!("fixtures/user_reviews.html");
const WEB_STATEMENTS: &str = include_str!("fixtures/user_statements.html");
const ALIVE: &str = include_str!("fixtures/alive.json");

/// What the web code asks for one list of a member: the page, then the list.
fn web_collection(nick: &str, list: &str) -> [String; 2] {
    [format!("/Users/{nick}/Collection/{list}"), "/action/wardrobe/get_wardrobe.php".to_string()]
}

struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// Whether the session token is refused from now on.
    refuse: AtomicBool,
}

impl Site {
    /// What was asked since the last call, which is forgotten.
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

/// The recorded profile of another member, under another nickname and with other values.
fn other_as(nick: &str, changes: &[(&str, &str)]) -> String {
    let mut profile = PROFILE_OTHER.replace("member1", nick);
    for (from, to) in changes {
        assert!(profile.contains(from), "the fixture has no {from}");
        profile = profile.replace(from, to);
    }
    profile
}

/// What `GET /users/{nick}` answers.
fn profile_of(nick: &str) -> Option<String> {
    let nick = nick.to_lowercase();
    let nick = nick.as_str();
    match nick {
        "testuser" => Some(PROFILE.to_string()),
        "member1" => Some(PROFILE_OTHER.to_string()),
        // Not recorded: a member with texts, photos and one perfume.
        "member2" => Some(other_as(
            nick,
            &[
                (r#""reviews": 0"#, r#""reviews": 3"#),
                (r#""statements": 0"#, r#""statements": 12"#),
                (r#""photos": 0"#, r#""photos": 2"#),
                (r#""have": 0"#, r#""have": 1"#),
            ],
        )),
        // Not recorded: a member with a custom list.
        "member3" => Some(other_as(nick, &[(r#""lists": []"#, CUSTOM_LISTS), (r#""have": 0"#, r#""have": 2"#)])),
        // Assumed: an answer about another member than the one asked for.
        "member4" => Some(PROFILE_OTHER.to_string()),
        // Assumed: a perfume worn that lacks the slug of its brand.
        "member5" => Some(other_as(nick, &[(r#""wearing": null"#, r#""wearing": {"id": 9438, "p_name": "Sauvage", "p_name_url": "Sauvage"}"#)])),
        // A list said to hold two perfumes, of which the interface then hands none.
        "member6" => Some(other_as(nick, &[(r#""have": 0"#, r#""have": 2"#)])),
        _ => None,
    }
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    // The token of the forms of the web site is asked once per client: it is not recorded.
    if uri.path() == "/api/alive" {
        return (StatusCode::OK, ALIVE).into_response();
    }
    site.asked.lock().unwrap().push(asked.clone());
    let json = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text.to_string()).into_response();
    let html = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], text.to_string()).into_response();
    let Some(call) = uri.path().strip_prefix("/app/v1/") else {
        // The web site: one member, whatever the nickname but `ghost`, whom nobody knows.
        return match uri.path().split('/').collect::<Vec<_>>().as_slice() {
            ["", "action", "wardrobe", "get_wardrobe.php"] => html(WEB_WARDROBE),
            ["", "Users", "ghost", ..] => (StatusCode::NOT_FOUND, "<html><body>Not found</body></html>").into_response(),
            ["", "Users", _] => html(WEB_PROFILE),
            ["", "Users", _, "Collection", _] => html(WEB_COLLECTION),
            ["", "Users", _, "reviews"] => html(WEB_REVIEWS),
            ["", "Users", _, "statements"] => html(WEB_STATEMENTS),
            _ => (StatusCode::NOT_FOUND, "<html><body>Not found</body></html>").into_response(),
        };
    };
    if call == "auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#);
    }
    if site.refuse.load(Ordering::SeqCst) || headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    // Assumed: the application takes a 404 for a member nobody knows; its body was not recorded.
    let unknown = || (StatusCode::NOT_FOUND, r#"{"status":"error","message":"User not found"}"#).into_response();
    match call.split('/').collect::<Vec<_>>().as_slice() {
        ["users", nick] => profile_of(nick).map(|profile| json(&profile)).unwrap_or_else(unknown),
        ["users", "testuser", "collection"] if asked.ends_with("?type=have") => json(COLLECTION),
        ["users", "member2", "collection"] if asked.ends_with("?type=have") => json(COLLECTION_EDP),
        // Assumed: the answer of another list than the one asked for.
        ["users", "testuser", "collection"] if asked.ends_with("?type=want") => json(COLLECTION),
        ["users", _, "collection"] => json(COLLECTION_EMPTY),
        _ => unknown(),
    }
}

/// A client of the fake site, without a session.
async fn client() -> (ParfumoClient, Arc<Site>) {
    let site = Arc::new(Site { asked: Mutex::new(Vec::new()), refuse: AtomicBool::new(false) });
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

/// A client logged in to the application interface of the fake site.
async fn logged_in() -> (ParfumoClient, Arc<Site>) {
    let (client, site) = client().await;
    client.app_login("someone@example.org", "pw").await.unwrap();
    site.take();
    (client, site)
}

#[tokio::test]
async fn a_public_profile_is_read_in_one_call() {
    let (c, site) = logged_in().await;
    let p = c.member_profile("testuser", true).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/users/testuser"], "no web page is read");

    assert_eq!((p.nick.as_str(), p.member_id, p.gender.as_deref()), ("testuser", Some(1001), Some("men")));
    assert_eq!(p.url, format!("{}/Users/testuser", c.base_url()));
    assert_eq!((p.country.as_deref(), p.points, p.followers), (Some("Pitcairn"), Some(0), Some(0)), "a zero the interface states is a zero");
    assert_eq!(p.member_since.as_deref(), Some("2026-01-01"), "the day of the registration in UTC, not the date the page writes");

    let worn = p.wearing.as_ref().unwrap();
    assert_eq!(
        (worn.id.as_str(), worn.numeric_id, worn.name.as_str()),
        ("Giorgio_Armani/emporio-armani-stronger-with-you-intensely-1", Some(120220), "Emporio Armani - Stronger With You Intensely")
    );
    assert_eq!(worn.url, format!("{}/Perfumes/Giorgio_Armani/emporio-armani-stronger-with-you-intensely-1", c.base_url()));

    // The lists that hold perfumes, under the ids and the names of the web site.
    let lists: Vec<_> = p.collections.iter().map(|l| (l.id.as_str(), l.name.as_str(), l.count)).collect();
    assert_eq!(lists, [("I_have", "I have", Some(2)), ("Wish_List", "Wish List", Some(1))]);
    assert_eq!(p.collections[1].url, format!("{}/Users/testuser/Collection/Wish_List", c.base_url()));

    // What the interface does not tell stays empty.
    assert_eq!((p.rank, p.activity, p.signature_scent), (None, None, None));
    assert!(p.achievements.is_empty() && p.counters.is_empty(), "no tab for a zero, as on the page");

    // The perfume worn gave its number: it is then read without being looked up.
    let _ = c.get_perfume("Giorgio_Armani/emporio-armani-stronger-with-you-intensely-1").await;
    assert_eq!(site.take(), ["/app/v1/perfume_details/120220"]);
}

#[tokio::test]
async fn a_reference_of_any_form_reaches_the_member_and_empty_lists_are_not_listed() {
    let (c, site) = logged_in().await;
    for reference in ["member1", "@member1", "/Users/member1/Collection/I_have", &format!("{}/Users/member1", c.base_url())] {
        let p = c.member_profile(reference, true).await.unwrap();
        assert_eq!(site.take(), ["/app/v1/users/member1"], "{reference}");
        assert_eq!((p.nick.as_str(), p.member_id, p.points, p.country.as_deref()), ("member1", Some(1002), Some(275), Some("Germany")));
        assert!(p.wearing.is_none() && p.collections.is_empty() && p.counters.is_empty());
    }
    // The letter case of the reference does not matter: the nickname is the one of the answer.
    let p = c.member_profile("TestUser", true).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/users/TestUser"]);
    assert_eq!((p.nick.as_str(), p.url), ("testuser", format!("{}/Users/testuser", c.base_url())));
}

#[tokio::test]
async fn the_tabs_the_interface_counts_are_the_counters() {
    let (c, site) = logged_in().await;
    let p = c.member_profile("member2", true).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/users/member2"]);
    let counters: Vec<_> = p.counters.iter().map(|t| (t.label.as_str(), t.count, t.url.clone())).collect();
    assert_eq!(
        counters,
        [
            ("Reviews", 3, Some(format!("{}/Users/member2/reviews", c.base_url()))),
            ("Statements", 12, Some(format!("{}/Users/member2/statements", c.base_url()))),
            ("Photos", 2, None),
        ],
        "the page of the photos is not told"
    );
}

#[tokio::test]
async fn the_uncached_read_keeps_the_web_page() {
    let (c, site) = logged_in().await;
    // The read of the account's own tools: they need the signature scent, which only the page has.
    let p = c.member_profile("testuser", false).await.unwrap();
    assert_eq!(site.take(), ["/Users/testuser"], "the interface is not asked");
    assert!(p.signature_scent.is_some() && p.rank.is_some());
}

#[tokio::test]
async fn a_profile_the_interface_cannot_tell_whole_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    // A custom list (never recorded), an answer about someone else, a perfume worn that cannot be read.
    for nick in ["member3", "member4", "member5"] {
        let p = c.member_profile(nick, true).await.unwrap();
        assert_eq!(site.take(), [format!("/app/v1/users/{nick}"), format!("/Users/{nick}")], "{nick}");
        assert_eq!(p.nick, "member273", "the page of the web site");
        assert!(p.collections.iter().any(|l| l.id == "1922"), "with its custom lists");
    }
    // A nickname the interface does not know: the web site words the refusal.
    let e = c.member_profile("ghost", true).await.unwrap_err();
    assert_eq!(site.take(), ["/app/v1/users/ghost", "/Users/ghost"]);
    assert!(matches!(&e, ClientError::NotFound(what) if what == "member 'ghost'"), "{e:?}");
}

#[tokio::test]
async fn a_fixed_list_is_read_whole_in_two_calls() {
    let (c, site) = logged_in().await;
    let (call, items) = c.member_collection("testuser", "I_have").await.unwrap();
    assert_eq!(site.take(), ["/app/v1/users/testuser", "/app/v1/users/testuser/collection?type=have"], "no web page is read");

    // What the page of the list says of it: the member, the list under both of its names, its size.
    assert_eq!(
        (call.member_id, call.nick.as_str(), call.list_type.as_str(), call.type_url.as_str(), call.total.as_str()),
        (1001, "testuser", "have", "I_have", "2")
    );
    assert_eq!(items.len(), 2);
    let hawas = &items[0];
    assert_eq!((hawas.p_id, hawas.name.as_str(), hawas.brand.as_deref()), (229262, "Hawas Ice / هوس Ice", Some("Rasasi")));
    assert_eq!(hawas.perfume_id.as_deref(), Some("Rasasi/hawas-ice-ice"));
    assert_eq!(hawas.url, Some(format!("{}/Perfumes/Rasasi/hawas-ice-ice", c.base_url())));
    assert!(hawas.image_url.as_deref().unwrap().starts_with("https://media.parfumo.com/perfumes/"));
    assert_eq!(items[1].perfume_id.as_deref(), Some("Giorgio_Armani/emporio-armani-stronger-with-you-intensely-1"));
    assert!(items.iter().all(|i| i.entry_id == 0), "the interface does not number the entries");

    // Such an entry is complete: resolving it asks nothing, and its perfume is read by its number.
    let mut entry = items[0].clone();
    c.resolve_collection_item(&mut entry).await.unwrap();
    assert_eq!(entry, items[0]);
    let _ = c.get_perfume("Rasasi/hawas-ice-ice").await;
    assert_eq!(site.take(), ["/app/v1/perfume_details/229262"]);

    // The letter case of the list does not matter, and an empty list is an empty answer.
    let (call, items) = c.member_collection("member1", "i_have").await.unwrap();
    assert_eq!(site.take(), ["/app/v1/users/member1", "/app/v1/users/member1/collection?type=have"]);
    assert_eq!((call.member_id, call.type_url.as_str(), call.total.as_str(), items.len()), (1002, "I_have", "0", 0));
}

#[tokio::test]
async fn the_concentration_is_part_of_the_name_as_on_the_web_site() {
    let (c, site) = logged_in().await;
    let (_, items) = c.member_collection("member2", "I_have").await.unwrap();
    assert_eq!(site.take(), ["/app/v1/users/member2", "/app/v1/users/member2/collection?type=have"]);
    assert_eq!((items[0].name.as_str(), items[0].perfume_id.as_deref()), ("Sauvage Eau de Parfum", Some("Dior/Sauvage_Eau_de_Parfum")));
}

#[tokio::test]
async fn a_list_the_interface_cannot_tell_whole_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    // A custom list and a segment the interface has no type for: it is not asked at all.
    for list in ["1922", "Somewhere"] {
        let (call, items) = c.member_collection("member1", list).await.unwrap();
        assert_eq!(site.take(), web_collection("member1", list), "{list}");
        assert_eq!((call.member_id, items.len()), (6467, 9), "the list of the web site");
        assert!(items.iter().all(|i| i.entry_id > 0 && i.perfume_id.is_none()));
    }
    // The answer of another list than asked, and fewer perfumes than the profile counts.
    for (nick, list, kind) in [("testuser", "Wish_List", "want"), ("member6", "I_have", "have")] {
        let (call, _) = c.member_collection(nick, list).await.unwrap();
        let mut expected = vec![format!("/app/v1/users/{nick}"), format!("/app/v1/users/{nick}/collection?type={kind}")];
        expected.extend(web_collection(nick, list));
        assert_eq!(site.take(), expected);
        assert_eq!(call.member_id, 6467);
    }
    // A member the interface does not know: the web site words the refusal.
    let e = c.member_collection("ghost", "I_have").await.unwrap_err();
    assert_eq!(site.take(), ["/app/v1/users/ghost", "/Users/ghost/Collection/I_have"]);
    assert!(matches!(&e, ClientError::NotFound(what) if what == "collection 'I_have' of member 'ghost'"), "{e:?}");
    // A list name that cannot be one is refused before anything is asked.
    assert!(matches!(c.member_collection("member1", "I have").await, Err(ClientError::InvalidInput(_))));
    assert!(site.take().is_empty());
}

#[tokio::test]
async fn the_texts_of_a_member_stay_on_the_web_pages() {
    let (c, site) = logged_in().await;
    // The interface was only seen to answer them empty: the shape of a row is not known.
    let reviews = c.member_texts_page("member1", TextKind::Review, 1).await.unwrap();
    assert_eq!(site.take(), ["/Users/member1/reviews"]);
    assert!(!reviews.items.is_empty() && reviews.items.iter().all(|t| !t.perfume_id.is_empty()));
    let statements = c.member_texts_page("member1", TextKind::Statement, 1).await.unwrap();
    assert_eq!(site.take(), ["/Users/member1/statements"]);
    assert!(!statements.items.is_empty());
}

#[tokio::test]
async fn without_a_session_nothing_is_asked_of_the_interface() {
    let (c, site) = client().await;
    assert!(!c.has_app_session());
    let p = c.member_profile("testuser", true).await.unwrap();
    assert_eq!(p.nick, "member273");
    let (call, items) = c.member_collection("testuser", "I_have").await.unwrap();
    assert_eq!((call.member_id, items.len()), (6467, 9));
    c.member_texts_page("testuser", TextKind::Review, 1).await.unwrap();
    let mut expected = vec!["/Users/testuser".to_string()];
    expected.extend(web_collection("testuser", "I_have"));
    expected.push("/Users/testuser/reviews".to_string());
    assert_eq!(site.take(), expected, "the web site only");
}

#[tokio::test]
async fn a_refused_token_is_not_sent_again() {
    let (c, site) = logged_in().await;
    site.refuse.store(true, Ordering::SeqCst);
    // One refusal, then the web page answers the same read at once.
    let p = c.member_profile("testuser", true).await.unwrap();
    assert_eq!(site.take(), ["/app/v1/users/testuser", "/Users/testuser"]);
    assert_eq!(p.nick, "member273");
    // The token is forgotten: the next reads ask the web site only.
    assert!(!c.has_app_session());
    c.member_profile("testuser", true).await.unwrap();
    c.member_collection("testuser", "I_have").await.unwrap();
    assert!(site.take().iter().all(|asked| !asked.starts_with("/app/v1/")));
}
