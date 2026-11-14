//! The member's own nickname, ratings and classification of a perfume through the
//! application interface, against answers recorded on the real site
//! (`tests/fixtures/app_own_*.json`). A local fake serves them, and the web page and panels
//! the client falls back to, and records what is asked.

use std::{
    collections::{BTreeSet, HashMap},
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
use parfumo_client::{ClassChoice, ClientConfig, ClientError, GlobalLimiter, MemberRatings, ParfumoClient, calls, parse};
use serde_json::{Value, json};

/// `GET /auth/me`, cut down to what names the member, every value replaced.
const ME: &str = include_str!("fixtures/app_own_me.json");
/// `GET /rating?p_id=120220`: the five scales rated.
const RATING: &str = include_str!("fixtures/app_own_rating.json");
/// `GET /rating?p_id=229262`: a perfume the member did not rate.
const RATING_NONE: &str = include_str!("fixtures/app_own_rating_none.json");
/// `GET /classification?p_id=9438`: the form, without any vote of the member.
const CLASSIFICATION: &str = include_str!("fixtures/app_own_classification.json");
/// The web site: the account page that names the member, and the two panels of a perfume
/// (scent rated 7.5 and nothing else; one vote, for `Fresh`).
const WEB_ACCOUNT: &str = include_str!("fixtures/auth_settings_notifications.html");
const WEB_RATE_PANEL: &str = include_str!("fixtures/auth_frag_rate_panel.html");
const WEB_CLASSIFY_PANEL: &str = include_str!("fixtures/auth_frag_classify_panel.html");

const RATED: u64 = 120_220;
const NOT_RATED: u64 = 229_262;
const NO_VOTE: u64 = 9438;

// What the site answers in the cases below was not recorded: the fake stands for what the
// client must withstand, not for what the site is known to do.
/// Two scales rated, one as a number and one as text; a third named without a value.
const PARTLY_RATED: u64 = 700_001;
/// A scale at `0`, which the application shows as no rating.
const RATED_ZERO: u64 = 700_002;
/// An answer made as to a visitor.
const AS_VISITOR: u64 = 700_003;
/// A scale under a name the model does not know.
const OTHER_SCALE: u64 = 700_004;
/// A rating past the top of the scale.
const PAST_THE_SCALE: u64 = 700_005;
/// An answer without the ratings.
const NO_RATINGS: u64 = 700_006;
/// The form with a vote of the member, as the application reads it (`voted` at `1`).
const VOTED: u64 = 700_011;
/// The form of a perfume whose classification is locked.
const LOCKED: u64 = 700_012;
/// An error object, answered with status 200.
const NOT_A_FORM: u64 = 700_013;
/// A form with a label that lost its identifier.
const BROKEN_FORM: u64 = 700_014;
/// The whole form, under a status other than success.
const NOT_A_SUCCESS: u64 = 700_015;
/// A form with a label that says nothing of the member's vote.
const NO_FLAG: u64 = 700_016;
/// A number the interface answers 404 to, on both calls.
const UNKNOWN: u64 = 999_999;

#[derive(Default)]
struct Site {
    /// Method, path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// Whether the token is refused from now on.
    refusing: AtomicBool,
    /// The nickname `GET /auth/me` answers instead of the recorded one.
    nick: Mutex<Option<String>>,
}

/// The recorded answer of a rated perfume, with other ratings.
fn rating(ratings: Value) -> String {
    let mut answer: Value = serde_json::from_str(RATING).unwrap();
    answer["ratings"] = ratings;
    answer.to_string()
}

/// The recorded form, changed by `change`.
fn form(change: impl FnOnce(&mut Value)) -> String {
    let mut answer: Value = serde_json::from_str(CLASSIFICATION).unwrap();
    change(&mut answer);
    answer.to_string()
}

/// The option of the recorded form that bears `label`.
fn option<'a>(form: &'a mut Value, label: &str) -> &'a mut Value {
    let groups = form["groups"].as_array_mut().unwrap();
    groups.iter_mut().flat_map(|g| g["options"].as_array_mut().unwrap().iter_mut()).find(|o| o["name"] == label).unwrap()
}

async fn handler(State(site): State<Arc<Site>>, method: Method, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let target = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(format!("{method} {target}"));
    let json = |text: String| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text).into_response();
    let html = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], text.to_string()).into_response();
    let path = uri.path();
    if !path.starts_with("/app/v1/") {
        return match (method.as_str(), path) {
            ("GET", "/api/alive") => json(r#"{"csrf_key":"fixturecsrfkey00"}"#.into()),
            ("GET", "/account/notifications") => html(WEB_ACCOUNT),
            ("POST", "/action/perfume/perfume_rate_panel.php") => html(WEB_RATE_PANEL),
            ("POST", "/action/perfume/perfume_classify_panel.php") => html(WEB_CLASSIFY_PANEL),
            _ => StatusCode::NOT_FOUND.into_response(),
        };
    }
    if path == "/app/v1/auth/login" {
        return json(r#"{"status":"success","auth_token":"tok"}"#.into());
    }
    if site.refusing.load(Ordering::SeqCst) || headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    // These are reads: the interface is never written to.
    if method != Method::GET {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let query: HashMap<String, String> = url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).into_owned().collect();
    let p_id = query.get("p_id").and_then(|p| p.parse::<u64>().ok()).unwrap_or_default();
    match (path, p_id) {
        ("/app/v1/auth/me", _) => match site.nick.lock().unwrap().clone() {
            Some(nick) => json(ME.replace("testuser", &nick)),
            None => json(ME.into()),
        },
        ("/app/v1/rating", RATED) => json(RATING.into()),
        ("/app/v1/rating", NOT_RATED) => json(RATING_NONE.into()),
        ("/app/v1/rating", PARTLY_RATED) => json(rating(json!({"scent": 75, "durability": null, "pricing": "100"}))),
        ("/app/v1/rating", RATED_ZERO) => json(rating(json!({"scent": "80", "durability": "0", "sillage": "70", "bottle": "60", "pricing": "90"}))),
        ("/app/v1/rating", AS_VISITOR) => json(RATING_NONE.replace("\"login_status\": 1", "\"login_status\": 0")),
        ("/app/v1/rating", OTHER_SCALE) => json(rating(json!({"scent": "80", "longevity": "90"}))),
        ("/app/v1/rating", PAST_THE_SCALE) => json(rating(json!({"scent": "800"}))),
        ("/app/v1/rating", NO_RATINGS) => json(r#"{"login_status":1,"nick_name":"testuser","user_id":"1"}"#.into()),
        ("/app/v1/classification", NO_VOTE) => json(CLASSIFICATION.into()),
        ("/app/v1/classification", VOTED) => json(form(|f| option(f, "Fresh")["voted"] = 1.into())),
        ("/app/v1/classification", LOCKED) => json(form(|f| f["locked"] = 1.into())),
        ("/app/v1/classification", NOT_A_FORM) => json(r#"{"status":"error","message":"Perfume not found"}"#.into()),
        ("/app/v1/classification", BROKEN_FORM) => json(form(|f| {
            option(f, "Fresh").as_object_mut().unwrap().remove("ct");
        })),
        ("/app/v1/classification", NOT_A_SUCCESS) => json(form(|f| f["status"] = "pending".into())),
        ("/app/v1/classification", NO_FLAG) => json(form(|f| {
            option(f, "Winter").as_object_mut().unwrap().remove("voted");
        })),
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

/// The ratings of the web panel of the fake: the scent alone.
fn web_ratings() -> MemberRatings {
    MemberRatings { scent: Some(7.5), ..MemberRatings::default() }
}

fn voted(choices: &[ClassChoice]) -> Vec<&str> {
    choices.iter().filter(|c| c.voted).map(|c| c.label.as_str()).collect()
}

#[tokio::test]
async fn the_nickname_is_read_from_the_session_once() {
    let (c, site) = logged_in().await;
    assert_eq!(c.own_nick().await.unwrap(), "testuser");
    assert_eq!(asked(&site), ["GET /app/v1/auth/me"], "one call, no web page");
    assert_eq!(c.own_nick().await.unwrap(), "testuser");
    assert!(asked(&site).is_empty(), "kept for the session");
}

#[tokio::test]
async fn a_nickname_that_cannot_name_a_page_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    *site.nick.lock().unwrap() = Some("two words".into());
    assert_eq!(c.own_nick().await.unwrap(), "testuser", "the one the account page shows");
    assert_eq!(asked(&site), ["GET /app/v1/auth/me", "GET /account/notifications"]);
}

#[tokio::test]
async fn ratings_out_of_a_hundred_are_given_out_of_ten() {
    let (c, site) = logged_in().await;
    let r = c.my_ratings_of(RATED).await.unwrap();
    assert_eq!(asked(&site), ["GET /app/v1/rating?p_id=120220"], "one call, no web panel");
    assert_eq!((r.scent, r.durability, r.sillage, r.bottle, r.pricing), (Some(8.0), Some(9.0), Some(7.0), Some(6.0), Some(9.0)));
    // The tools compare a rating with the value they would send, in tenths.
    for (scale, tenths) in [("scent", 80_u32), ("durability", 90), ("sillage", 70), ("bottle", 60), ("pricing", 90)] {
        assert_eq!(r.get(scale), Some(tenths as f32 / 10.0), "{scale}");
    }

    let none = c.my_ratings_of(NOT_RATED).await.unwrap();
    assert_eq!(asked(&site), ["GET /app/v1/rating?p_id=229262"]);
    assert_eq!(none, MemberRatings::default(), "an empty list: not rated on any scale");
}

#[tokio::test]
async fn a_scale_without_a_rating_stays_empty() {
    let (c, site) = logged_in().await;
    let r = c.my_ratings_of(PARTLY_RATED).await.unwrap();
    assert_eq!(asked(&site), ["GET /app/v1/rating?p_id=700001"]);
    assert_eq!((r.scent, r.pricing), (Some(7.5), Some(10.0)), "a number or a text, with the half point of the web panel");
    assert_eq!((r.durability, r.sillage, r.bottle), (None, None, None), "named without a value, or not named");
}

#[tokio::test]
async fn ratings_whose_meaning_was_not_recorded_are_read_on_the_web_panel() {
    let (c, site) = logged_in().await;
    // A zero is no rating for the application, and may be a rating of zero for the web panel.
    assert_eq!(c.my_ratings_of(RATED_ZERO).await.unwrap(), web_ratings());
    assert_eq!(asked(&site), ["GET /app/v1/rating?p_id=700002", "GET /api/alive", "POST /action/perfume/perfume_rate_panel.php"]);
    for p_id in [AS_VISITOR, OTHER_SCALE, PAST_THE_SCALE, UNKNOWN] {
        assert_eq!(c.my_ratings_of(p_id).await.unwrap(), web_ratings(), "{p_id}");
        assert_eq!(asked(&site), [format!("GET /app/v1/rating?p_id={p_id}"), "POST /action/perfume/perfume_rate_panel.php".into()], "{p_id}");
    }
    // An answer without the ratings is an error, not an absence of ratings.
    assert!(matches!(c.my_ratings_of(NO_RATINGS).await, Err(ClientError::ParserOutdated(_))));
    assert_eq!(asked(&site), ["GET /app/v1/rating?p_id=700006"]);
}

#[tokio::test]
async fn the_classification_form_names_the_labels_as_the_web_panel_does() {
    let (c, site) = logged_in().await;
    let choices = c.my_classification_of(NO_VOTE).await.unwrap();
    assert_eq!(asked(&site), ["GET /app/v1/classification?p_id=9438"], "one call, no web panel");
    assert_eq!(choices.len(), 6 + 4 + 4 + 21);
    assert!(voted(&choices).is_empty(), "the member voted for nothing");

    let mut groups: Vec<&str> = choices.iter().map(|c| c.group.as_str()).collect();
    groups.dedup();
    assert_eq!(groups, ["Type", "Style", "Season", "Occasion"], "the order of the web panel, each group in one run");
    let labels: Vec<&str> = choices.iter().filter(|c| c.group == "Season").map(|c| c.label.as_str()).collect();
    assert_eq!(labels, ["Spring", "Summer", "Fall", "Winter"]);

    // `c` and `ct` are the numbers of the web panel, which the vote of the web site takes:
    // the form is the one the web panel gives, label for label, but for the vote it shows.
    let web = parse::parse_classify_panel(WEB_CLASSIFY_PANEL).unwrap();
    let web_without_vote: Vec<ClassChoice> = web.iter().map(|c| ClassChoice { voted: false, ..c.clone() }).collect();
    assert_eq!(choices, web_without_vote);
    let ids: BTreeSet<(u64, u64)> = choices.iter().map(|c| (c.group_id, c.label_id)).collect();
    assert_eq!(ids.len(), choices.len(), "no two labels share their identifiers");
    let night = choices.iter().find(|c| c.label == "Night Out").unwrap();
    let vote = calls::classification_vote(NO_VOTE, night.group_id, night.label_id, true);
    assert_eq!((vote.form["c"].as_str(), vote.form["ct"].as_str()), ("2", "7"));
}

#[tokio::test]
async fn a_form_whose_meaning_was_not_recorded_is_read_on_the_web_panel() {
    let (c, site) = logged_in().await;
    // Only a form without any vote was recorded: one with a vote is read where votes are known.
    let choices = c.my_classification_of(VOTED).await.unwrap();
    assert_eq!(asked(&site), ["GET /app/v1/classification?p_id=700011", "GET /api/alive", "POST /action/perfume/perfume_classify_panel.php"]);
    assert_eq!(voted(&choices), ["Fresh"], "the vote the web panel shows");
    for p_id in [LOCKED, NOT_A_FORM, NOT_A_SUCCESS, NO_FLAG, UNKNOWN] {
        assert_eq!(voted(&c.my_classification_of(p_id).await.unwrap()), ["Fresh"], "{p_id}");
        assert_eq!(
            asked(&site),
            [format!("GET /app/v1/classification?p_id={p_id}"), "POST /action/perfume/perfume_classify_panel.php".into()],
            "{p_id}"
        );
    }
    // A form that lost a field is an error, not a shorter form.
    assert!(matches!(c.my_classification_of(BROKEN_FORM).await, Err(ClientError::ParserOutdated(_))));
    assert_eq!(asked(&site), ["GET /app/v1/classification?p_id=700014"]);
}

#[tokio::test]
async fn a_refused_token_is_not_sent_again_and_the_web_site_answers() {
    let (c, site) = logged_in().await;
    site.refusing.store(true, Ordering::SeqCst);
    assert_eq!(c.my_ratings_of(RATED).await.unwrap(), web_ratings(), "the read is answered by the web site at once");
    assert_eq!(asked(&site), ["GET /app/v1/rating?p_id=120220", "GET /api/alive", "POST /action/perfume/perfume_rate_panel.php"]);
    assert!(!c.has_app_session(), "the refused token is forgotten");

    assert_eq!(c.own_nick().await.unwrap(), "testuser");
    assert_eq!(voted(&c.my_classification_of(NO_VOTE).await.unwrap()), ["Fresh"]);
    assert_eq!(asked(&site), ["GET /account/notifications", "POST /action/perfume/perfume_classify_panel.php"]);
}

#[tokio::test]
async fn each_read_whose_call_is_refused_is_answered_by_the_web_site() {
    // The nickname.
    let (c, site) = logged_in().await;
    site.refusing.store(true, Ordering::SeqCst);
    assert_eq!(c.own_nick().await.unwrap(), "testuser");
    assert_eq!(asked(&site), ["GET /app/v1/auth/me", "GET /account/notifications"]);
    assert!(!c.has_app_session());
    // The one the web page gave is kept as well: nothing more is asked.
    assert_eq!(c.own_nick().await.unwrap(), "testuser");
    assert!(asked(&site).is_empty());

    // The classification.
    let (c, site) = logged_in().await;
    site.refusing.store(true, Ordering::SeqCst);
    assert_eq!(voted(&c.my_classification_of(NO_VOTE).await.unwrap()), ["Fresh"]);
    assert_eq!(asked(&site), ["GET /app/v1/classification?p_id=9438", "GET /api/alive", "POST /action/perfume/perfume_classify_panel.php"]);
    assert!(!c.has_app_session());
}

#[tokio::test]
async fn the_nickname_of_the_session_names_the_pages_the_web_site_is_asked_for() {
    let (c, site) = logged_in().await;
    // The page itself is not served by the fake: what matters is the address asked.
    let _ = c.my_ratings_page("scent", "d_desc", 1).await;
    let seen = asked(&site);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0], "GET /app/v1/auth/me");
    assert!(seen[1].starts_with("GET /Users/testuser/"), "{seen:?}");
}

#[tokio::test]
async fn without_a_session_nothing_is_asked_of_the_interface() {
    let (c, site) = logged_in().await;
    c.forget_app_session();
    assert_eq!(c.own_nick().await.unwrap(), "testuser");
    assert_eq!(c.my_ratings_of(RATED).await.unwrap(), web_ratings());
    assert_eq!(voted(&c.my_classification_of(NO_VOTE).await.unwrap()), ["Fresh"]);
    assert_eq!(
        asked(&site),
        [
            "GET /account/notifications",
            "GET /api/alive",
            "POST /action/perfume/perfume_rate_panel.php",
            "POST /action/perfume/perfume_classify_panel.php"
        ]
    );
}
