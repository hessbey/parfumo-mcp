//! The wear log through the application interface, against answers recorded on the real
//! site (`tests/fixtures/app_wears_*.json`). A local fake serves them, and the web pages
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
use parfumo_client::{ClientConfig, ClientError, GlobalLimiter, ParfumoClient, Wear};
use serde_json::{Value, json};

/// `GET /tracker_calendar?y=2026&m=11`: one wear, on the 13th.
const CALENDAR: &str = include_str!("fixtures/app_wears_calendar.json");
/// `GET /tracker_calendar` of the same month, before the wear was logged.
const CALENDAR_EMPTY: &str = include_str!("fixtures/app_wears_calendar_empty.json");
/// `GET /tracker_overview?offset=0&limit=20`: the same wear.
const OVERVIEW: &str = include_str!("fixtures/app_wears_overview.json");
/// The same call, before the wear was logged.
const OVERVIEW_EMPTY: &str = include_str!("fixtures/app_wears_overview_empty.json");
const PERFUME_DETAILS: &str = include_str!("fixtures/app_perfume_details.json");
const WEB_ALIVE: &str = include_str!("fixtures/alive.json");
/// The web site: one wear of another perfume, numbered 22205264.
const WEB_MONTH: &str = include_str!("fixtures/auth_frag_calendar_month.html");
const WEB_TRACKER: &str = include_str!("fixtures/auth_tracker.html");

/// The wear of the recorded answers.
const L_ID: u64 = 22_231_107;
const PERFUME: &str = "Giorgio_Armani/emporio-armani-stronger-with-you-intensely-1";
/// Its instant: 2026-11-13 20:42:43 UTC, shown to the member as 09:42 PM.
const INSTANT: i64 = 1_794_602_563;

const NOVEMBER: &str = "/app/v1/tracker_calendar?y=2026&m=11";
const FIRST_WINDOW: &str = "/app/v1/tracker_overview?offset=0&limit=20";
const WEB_MONTH_PATH: &str = "/api/assistant/calendar_month";
const WEB_TRACKER_PATH: &str = "/assistant/tracker";

/// What `/tracker_overview` answers.
enum History {
    /// An answer as it is, whatever the window asked.
    Answer(String),
    // The answers recorded hold one wear: how the site cuts a longer history is what the
    // paging of the application implies, not something measured.
    /// The windows of these rows.
    Rows(Vec<Value>),
    /// The first window of these rows, whatever the one asked.
    FirstWindowOf(Vec<Value>),
    /// The windows of these rows, each said to be followed by more.
    Endless(Vec<Value>),
}

struct Site {
    /// Path and query of every request, in order.
    asked: Mutex<Vec<String>>,
    /// Whether the token is refused from now on.
    refusing: AtomicBool,
    /// What `/tracker_calendar` answers, whatever the month asked.
    calendar: Mutex<String>,
    history: Mutex<History>,
}

impl Site {
    fn answer_month(&self, answer: impl ToString) {
        *self.calendar.lock().unwrap() = answer.to_string();
    }

    fn answer_history(&self, history: History) {
        *self.history.lock().unwrap() = history;
    }
}

/// The recorded wear as another one. Not recorded: written in the recorded shape.
fn row(l_id: u64, change: impl FnOnce(&mut Value)) -> Value {
    let mut row = serde_json::from_str::<Value>(CALENDAR).unwrap()["items"][0].clone();
    row["l_id"] = l_id.into();
    change(&mut row);
    row
}

/// A history of `n` wears, the most recent first.
fn rows(n: u64) -> Vec<Value> {
    (0..n).map(|i| row(L_ID - i, |_| {})).collect()
}

/// The recorded month with these wears, counted as the site counts them.
fn month_of(items: Vec<Value>) -> Value {
    let mut answer: Value = serde_json::from_str(CALENDAR).unwrap();
    answer["totals"]["wears"] = items.len().into();
    answer["items"] = items.into();
    answer
}

async fn handler(State(site): State<Arc<Site>>, uri: Uri, headers: HeaderMap, _body: String) -> Response {
    let asked = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    site.asked.lock().unwrap().push(asked);
    let json = |text: String| (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], text).into_response();
    let html = |text: &str| (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], text.to_string()).into_response();
    match uri.path() {
        // The web site.
        "/api/alive" => return json(WEB_ALIVE.into()),
        WEB_MONTH_PATH => return html(WEB_MONTH),
        WEB_TRACKER_PATH => return html(WEB_TRACKER),
        "/app/v1/auth/login" => return json(r#"{"status":"success","auth_token":"tok"}"#.into()),
        _ => {}
    }
    if site.refusing.load(Ordering::SeqCst) || headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok") {
        return (StatusCode::UNAUTHORIZED, r#"{"status":"error","message":"Authorization header required"}"#).into_response();
    }
    let query: HashMap<String, String> = url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).into_owned().collect();
    let nr = |key: &str| query.get(key).and_then(|v| v.parse::<usize>().ok()).unwrap_or_default();
    match uri.path() {
        "/app/v1/tracker_calendar" => json(site.calendar.lock().unwrap().clone()),
        "/app/v1/tracker_overview" => {
            let (all, offset, endless) = match &*site.history.lock().unwrap() {
                History::Answer(answer) => return json(answer.clone()),
                History::Rows(all) => (all.clone(), nr("offset"), false),
                History::FirstWindowOf(all) => (all.clone(), 0, false),
                History::Endless(all) => (all.clone(), nr("offset"), true),
            };
            let window: Vec<Value> = all.iter().skip(offset).take(nr("limit")).cloned().collect();
            let has_more = endless || offset + window.len() < all.len();
            json(
                json!({"login_status": 1, "has_more": has_more, "total_count": all.len(), "offset": offset, "limit": nr("limit"), "items": window})
                    .to_string(),
            )
        }
        // One record, whatever the number.
        path if path.starts_with("/app/v1/perfume_details/") => json(PERFUME_DETAILS.into()),
        _ => (StatusCode::NOT_FOUND, r#"{"error":"Not found"}"#).into_response(),
    }
}

/// A client logged in to the application interface of a fake site, which answers what was recorded.
async fn logged_in() -> (ParfumoClient, Arc<Site>) {
    let site = Arc::new(Site {
        asked: Mutex::default(),
        refusing: AtomicBool::new(false),
        calendar: Mutex::new(CALENDAR.into()),
        history: Mutex::new(History::Answer(OVERVIEW.into())),
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

/// What was asked since the last look.
fn asked(site: &Site) -> Vec<String> {
    std::mem::take(&mut *site.asked.lock().unwrap())
}

/// The wear of the recorded answers, as the model must hold it.
fn recorded_wear(c: &ParfumoClient) -> Wear {
    Wear {
        wear_id: L_ID,
        perfume_id: PERFUME.into(),
        name: "Emporio Armani - Stronger With You Intensely".into(),
        brand: Some("Giorgio Armani".into()),
        url: format!("{}/Perfumes/{PERFUME}", c.base_url()),
        // The local time the site shows, as the web parser gives it: not the instant, which is 20:42 UTC.
        worn_at: Some("2026-11-13T21:42".into()),
        comment: None,
    }
}

#[tokio::test]
async fn a_month_is_read_in_one_call() {
    let (c, site) = logged_in().await;
    let wears = c.wears_of_month(2026, 11).await.unwrap();
    assert_eq!(asked(&site), [NOVEMBER], "one call, no web page");
    assert_eq!(wears, [recorded_wear(&c)]);

    // The row names the number of its perfume: its record is then read without looking it up.
    c.get_perfume(PERFUME).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/perfume_details/120220"]);

    // A month without a wear is an empty list, not a reason to ask the web site.
    site.answer_month(CALENDAR_EMPTY);
    assert_eq!(c.wears_of_month(2026, 11).await.unwrap(), []);
    assert_eq!(asked(&site), [NOVEMBER]);
}

#[tokio::test]
async fn the_history_is_read_in_one_call_when_it_fits_a_window() {
    let (c, site) = logged_in().await;
    let wears = c.wears_overview().await.unwrap();
    assert_eq!(asked(&site), [FIRST_WINDOW], "one call, no web page");
    assert_eq!(wears, [recorded_wear(&c)], "the same wear as in the calendar, under the same identifier");

    site.answer_history(History::Answer(OVERVIEW_EMPTY.into()));
    assert_eq!(c.wears_overview().await.unwrap(), []);
    assert_eq!(asked(&site), [FIRST_WINDOW]);
}

#[tokio::test]
async fn a_longer_history_is_read_window_after_window() {
    let (c, site) = logged_in().await;
    let window = |offset: u32| format!("/app/v1/tracker_overview?offset={offset}&limit=20");
    let ids = |wears: &[Wear]| wears.iter().map(|w| w.wear_id).collect::<Vec<_>>();

    site.answer_history(History::Rows(rows(45)));
    let wears = c.wears_overview().await.unwrap();
    assert_eq!(asked(&site), [window(0), window(20), window(40)], "each window starts after the rows already read");
    assert_eq!(ids(&wears), (0..45).map(|i| L_ID - i).collect::<Vec<_>>(), "every wear, in the order of the interface");

    // A history that ends with a window: the site says there is no more.
    site.answer_history(History::Rows(rows(40)));
    assert_eq!(c.wears_overview().await.unwrap().len(), 40);
    assert_eq!(asked(&site), [window(0), window(20)]);

    // The hundred most recent wears at most: five calls, however long the history.
    site.answer_history(History::Rows(rows(130)));
    let wears = c.wears_overview().await.unwrap();
    assert_eq!(asked(&site), [window(0), window(20), window(40), window(60), window(80)]);
    assert_eq!(ids(&wears), (0..100).map(|i| L_ID - i).collect::<Vec<_>>());

    // A wear logged between two calls shifts the windows: the row that comes twice is kept once.
    let mut shifted = rows(30);
    shifted.insert(20, shifted[19].clone());
    site.answer_history(History::Rows(shifted));
    let wears = c.wears_overview().await.unwrap();
    assert_eq!(asked(&site), [window(0), window(20)]);
    assert_eq!(ids(&wears), (0..30).map(|i| L_ID - i).collect::<Vec<_>>());

    // A window that brings nothing new ends the reading, whatever the site says of what follows.
    site.answer_history(History::Endless(rows(25)));
    let wears = c.wears_overview().await.unwrap();
    assert_eq!(asked(&site), [window(0), window(20), window(25)]);
    assert_eq!(ids(&wears), (0..25).map(|i| L_ID - i).collect::<Vec<_>>());
}

#[tokio::test]
async fn a_date_is_the_local_time_the_site_shows_or_nothing() {
    let (c, site) = logged_in().await;
    let shown = |text: &'static str, instant: Value| {
        move |row: &mut Value| {
            row["wear_date_readable"] = text.into();
            row["wear_date"] = instant;
        }
    };
    site.answer_month(month_of(vec![
        // The same instant for a member thirteen hours ahead of UTC, and one ten hours behind.
        row(1, shown("Saturday, 14/11/2026, 09:42 AM", INSTANT.into())),
        row(2, shown("Friday, 13/11/2026, 10:42 AM", INSTANT.into())),
        // Midnight and noon, as the web parser reads them.
        row(3, shown("Friday, 13/11/2026, 12:05 AM", INSTANT.into())),
        row(4, shown("Friday, 13/11/2026, 12:05 PM", INSTANT.into())),
        // The instant as text, which the interface also sends elsewhere.
        row(5, shown("Friday, 13/11/2026, 09:42 PM", INSTANT.to_string().into())),
        // No instant to check the text against: the text is taken as the web page's is.
        row(6, shown("Friday, 13/11/2026, 09:42 PM", Value::Null)),
        // 3 April 2026, 09:00 UTC, written month first: read as 4 March, which the instant rules out.
        row(7, shown("Friday, 04/03/2026, 09:00 AM", 1_775_206_800.into())),
        // Another form, and no text at all: the instant is not a local time and is not given as one.
        row(8, shown("A moment ago", INSTANT.into())),
        row(9, |row| row["wear_date_readable"] = Value::Null),
    ]));
    let wears = c.wears_of_month(2026, 11).await.unwrap();
    assert_eq!(asked(&site), [NOVEMBER], "a wear without a date is still a wear: the web site is not asked");
    let dates: Vec<_> = wears.iter().map(|w| (w.wear_id, w.worn_at.as_deref())).collect();
    assert_eq!(
        dates,
        [
            (1, Some("2026-11-14T09:42")),
            (2, Some("2026-11-13T10:42")),
            (3, Some("2026-11-13T00:05")),
            (4, Some("2026-11-13T12:05")),
            (5, Some("2026-11-13T21:42")),
            (6, Some("2026-11-13T21:42")),
            (7, None),
            (8, None),
            (9, None),
        ]
    );
}

#[tokio::test]
async fn the_note_of_a_wear_is_its_comment() {
    let (c, site) = logged_in().await;
    // Not recorded: the recorded wear has no note (`null`). A text is what the application displays.
    let noted = |text: &'static str| move |row: &mut Value| row["comment"] = text.into();
    site.answer_history(History::Rows(vec![row(1, noted("  at the office,\n in the morning ")), row(2, noted("a < b & c")), row(3, noted("  "))]));
    let wears = c.wears_overview().await.unwrap();
    let notes: Vec<_> = wears.iter().map(|w| w.comment.as_deref()).collect();
    assert_eq!(notes, [Some("at the office, in the morning"), Some("a < b & c"), None], "on one line as on the web page, and otherwise untouched");
}

#[tokio::test]
async fn a_perfume_is_named_as_the_web_page_names_it() {
    let (c, site) = logged_in().await;
    let named = |w: &Wear| (w.perfume_id.clone(), w.name.clone(), w.brand.clone());
    // Not recorded: the perfume of the recorded wear has no concentration. This is the
    // perfume of the web fixtures, in the shape the other answers of the interface give a
    // perfume that has one: `p_name` with the concentration in brackets, `con_ext_name` without.
    let sauvage = |p_name: &'static str, concentration: Value| {
        move |row: &mut Value| {
            (row["b_name"], row["b_name_url"], row["p_name_url"]) = ("Dior".into(), "Dior".into(), "Sauvage_Eau_de_Toilette".into());
            (row["p_name"], row["con_ext_name"], row["concentration"]) = (p_name.into(), "Sauvage".into(), concentration);
        }
    };
    site.answer_history(History::Rows(vec![row(1, sauvage("Sauvage (Eau de Toilette)", "Eau de Toilette".into()))]));
    let through_interface = c.wears_overview().await.unwrap();
    assert_eq!(asked(&site), [FIRST_WINDOW]);
    assert_eq!(named(&through_interface[0]), ("Dior/Sauvage_Eau_de_Toilette".into(), "Sauvage (Eau de Toilette)".into(), Some("Dior".into())));

    // A name that does not carry the concentration the row gives, in brackets, is not the
    // name of the web page: the wears are read there, where the same perfume bears the name read above.
    for other in ["Sauvage", "Sauvage Eau de Toilette"] {
        site.answer_history(History::Rows(vec![row(1, |_| {}), row(2, sauvage(other, "Eau de Toilette".into()))]));
        let from_web = c.wears_overview().await.unwrap();
        assert_eq!(asked(&site), [FIRST_WINDOW, WEB_TRACKER_PATH], "{other}");
        assert_eq!(named(&from_web[0]), named(&through_interface[0]), "{other}");
    }

    // No concentration, as `null` or as the empty text of neighbouring answers: the name is all there is.
    site.answer_history(History::Rows(vec![row(1, sauvage("Sauvage", Value::Null)), row(2, sauvage("Sauvage", "".into()))]));
    let names: Vec<_> = c.wears_overview().await.unwrap().into_iter().map(|w| w.name).collect();
    assert_eq!(asked(&site), [FIRST_WINDOW]);
    assert_eq!(names, ["Sauvage", "Sauvage"]);
}

#[tokio::test]
async fn what_is_not_surely_the_wears_asked_is_left_to_the_web_site() {
    let (c, site) = logged_in().await;
    let web_wear = |wears: Vec<Wear>| wears.iter().map(|w| (w.wear_id, w.perfume_id.clone())).collect::<Vec<_>>();
    let from_web = [(22_205_264, "Dior/Sauvage_Eau_de_Toilette".to_string())];

    // Another month than the one asked: the interface answers the current one when it does not understand.
    let wears = c.wears_of_month(2025, 3).await.unwrap();
    assert_eq!(asked(&site), ["/app/v1/tracker_calendar?y=2025&m=3", "/api/alive", WEB_MONTH_PATH]);
    assert_eq!(web_wear(wears), from_web, "the fragment of the web site, not the month the interface answered");

    let recorded = || serde_json::from_str::<Value>(CALENDAR).unwrap();
    let unsure: Vec<(&str, Value)> = vec![
        // The year and the month are each checked: one of them right is not the month asked.
        ("the month asked, of another year", {
            let mut answer = recorded();
            answer["year"] = 2025.into();
            answer
        }),
        ("another month of the year asked", {
            let mut answer = recorded();
            answer["month"] = 10.into();
            answer
        }),
        ("fewer rows than the month counts", {
            let mut answer = recorded();
            answer["totals"]["wears"] = 2.into();
            answer
        }),
        ("no count of the month", {
            let mut answer = recorded();
            answer["totals"] = Value::Null;
            answer
        }),
        ("no list", {
            let mut answer = recorded();
            answer["items"] = Value::Null;
            answer
        }),
        ("a member the interface does not recognise", {
            let mut answer = month_of(vec![]);
            answer["login_status"] = 0.into();
            answer
        }),
        ("a row without the identifier of the wear", month_of(vec![row(1, |_| {}), row(2, |row| row["l_id"] = Value::Null)])),
        ("a row without the address of the brand", month_of(vec![row(1, |row| row["b_name_url"] = Value::Null)])),
        ("a row without the address of the perfume", month_of(vec![row(1, |row| row["p_name_url"] = "".into())])),
        // `con_ext_name` is the name without its concentration: it does not stand in for the name.
        ("a row without a name", month_of(vec![row(1, |row| row["p_name"] = Value::Null)])),
        ("a note in a shape never seen", month_of(vec![row(1, |row| row["comment"] = json!({"text": "x"}))])),
    ];
    for (why, answer) in unsure {
        site.answer_month(answer);
        let wears = c.wears_of_month(2026, 11).await.unwrap();
        assert_eq!(asked(&site), [NOVEMBER, WEB_MONTH_PATH], "{why}");
        assert_eq!(web_wear(wears), from_web, "{why}");
    }

    // The history: a row that cannot be read, in the first window or a later one.
    let mut history = rows(25);
    history[22]["l_id"] = Value::Null;
    site.answer_history(History::Rows(history));
    assert_eq!(web_wear(c.wears_overview().await.unwrap()), from_web);
    assert_eq!(asked(&site), [FIRST_WINDOW, "/app/v1/tracker_overview?offset=20&limit=20", WEB_TRACKER_PATH]);
    // Another window than the one asked.
    site.answer_history(History::FirstWindowOf(rows(25)));
    assert_eq!(web_wear(c.wears_overview().await.unwrap()), from_web);
    assert_eq!(asked(&site), [FIRST_WINDOW, "/app/v1/tracker_overview?offset=20&limit=20", WEB_TRACKER_PATH]);
    site.answer_history(History::Answer(r#"{"login_status":0,"has_more":false,"total_count":0,"offset":0,"limit":20,"items":[]}"#.into()));
    assert_eq!(web_wear(c.wears_overview().await.unwrap()), from_web);
    assert_eq!(asked(&site), [FIRST_WINDOW, WEB_TRACKER_PATH]);
    // Whether the history goes on, in a shape never seen: a cut history is not taken for a whole one.
    for has_more in [json!(1), json!("1"), Value::Null] {
        let mut answer: Value = serde_json::from_str(OVERVIEW).unwrap();
        answer["has_more"] = has_more.clone();
        site.answer_history(History::Answer(answer.to_string()));
        assert_eq!(web_wear(c.wears_overview().await.unwrap()), from_web, "has_more: {has_more}");
        assert_eq!(asked(&site), [FIRST_WINDOW, WEB_TRACKER_PATH], "has_more: {has_more}");
    }
}

#[tokio::test]
async fn without_a_session_nothing_is_asked_of_the_interface() {
    let (c, site) = logged_in().await;
    c.forget_app_session();
    let month = c.wears_of_month(2026, 11).await.unwrap();
    assert_eq!(asked(&site), ["/api/alive", WEB_MONTH_PATH]);
    assert_eq!(
        (month[0].wear_id, month[0].worn_at.as_deref(), month[0].comment.as_deref()),
        (22_205_264, Some("2026-11-11T08:00"), Some("test note"))
    );
    let overview = c.wears_overview().await.unwrap();
    assert_eq!(asked(&site), [WEB_TRACKER_PATH]);
    assert_eq!((overview[0].wear_id, overview[0].worn_at.as_deref()), (22_205_264, Some("2026-11-12T07:21")));
}

#[tokio::test]
async fn a_refused_token_is_not_sent_again() {
    let (c, site) = logged_in().await;
    site.refusing.store(true, Ordering::SeqCst);
    assert_eq!(c.wears_overview().await.unwrap_err(), ClientError::LoginRequired);
    assert_eq!(asked(&site), [FIRST_WINDOW]);
    // The token is forgotten: the next reads are the ones of the web site.
    assert!(!c.has_app_session());
    c.wears_of_month(2026, 11).await.unwrap();
    c.wears_overview().await.unwrap();
    assert_eq!(asked(&site), ["/api/alive", WEB_MONTH_PATH, WEB_TRACKER_PATH]);
}
