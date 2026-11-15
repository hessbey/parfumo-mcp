//! The canary against a stand-in for both sides of Parfumo: the web site of the account
//! tests, and the application interface (`/app/v1`) with the answers recorded on the real
//! site. The stand-in keeps every request it receives, in order: what the canary asked, of
//! which side and with which token, is read from that log.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};
use parfumo_mcp_core::{
    db::User,
    testing::{
        link_account,
        site::{Site, spawn_site},
        test_config,
    },
};

use super::*;
use crate::tests::TestApp;

const RATE_PANEL: &str = "/action/perfume/perfume_rate_panel.php";
const CLASSIFY_PANEL: &str = "/action/perfume/perfume_classify_panel.php";

/// One request received by the stand-in.
#[derive(Clone, Debug)]
struct Asked {
    method: Method,
    /// Path and query.
    target: String,
    /// The token of the `Authorization: Bearer` header, if any.
    bearer: Option<String>,
    cookie: String,
}

impl Asked {
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    /// The call of the application interface, without its prefix; `None` for the web site.
    fn call(&self) -> Option<&str> {
        self.path().strip_prefix("/app/v1")
    }

    /// A call made as a member: every call of the interface but the login.
    fn member_call(&self) -> bool {
        self.call().is_some_and(|call| call != "/auth/login")
    }
}

#[derive(Clone)]
struct Parfumo {
    /// The web site of the account tests. It also answers the login of the application
    /// interface, and knows the tokens that are live.
    site: Site,
    upstream: String,
    http: reqwest::Client,
    asked: Arc<Mutex<Vec<Asked>>>,
    /// The search of the interface does not know the brand the canary asks for, and the
    /// web site has its page: the client then reads the web page.
    brand_on_the_web_only: Arc<AtomicBool>,
}

impl Parfumo {
    fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }

    fn forget_asked(&self) {
        self.asked.lock().unwrap().clear();
    }

    /// The live token of an account.
    fn token_of(&self, account: &str) -> String {
        self.site.app_live.lock().unwrap().iter().find(|(_, user)| *user == account).map(|(token, _)| token.clone()).unwrap()
    }

    /// The site ends every session of the application interface.
    fn end_app_sessions(&self) {
        self.site.app_live.lock().unwrap().clear();
    }
}

fn fixture(file: &str) -> String {
    std::fs::read_to_string(format!("{}/../parfumo-client/tests/fixtures/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// An answer recorded on the real site.
fn recorded(name: &str) -> String {
    fixture(&format!("app_{name}.json"))
}

/// The answers were recorded for the brand Creed and its perfume Aventus; the canary asks
/// for Dior and its Sauvage. These two names are changed, as text, and nothing else: every
/// field and its type stay those of the real site.
fn renamed(answer: String) -> String {
    answer.replace("\"Creed\"", "\"Dior\"").replace("\"aventus\"", "\"Sauvage_Eau_de_Toilette\"")
}

/// The application interface, with the recorded answers. Like the real one, it answers
/// nothing without a live token.
fn interface(parfumo: &Parfumo, call: &str, query: &str, bearer: Option<&str>) -> Response {
    let json = |status: StatusCode, text: String| (status, [(header::CONTENT_TYPE, "application/json")], text).into_response();
    if !bearer.is_some_and(|token| parfumo.site.app_live.lock().unwrap().contains_key(token)) {
        return json(StatusCode::UNAUTHORIZED, "{\"status\":\"error\",\"message\":\"Authorization header required\"}".into());
    }
    let asks = |param: &str| query.split('&').any(|pair| pair.starts_with(param));
    let answer = match call {
        "/auth/me" => recorded("own_me"),
        "/search" if parfumo.brand_on_the_web_only.load(Ordering::SeqCst) => recorded("search_creed"),
        "/search" => renamed(recorded("search_creed")),
        // The perfumes of one brand: the listing inside the recorded page of the brand.
        "/perfumes" if asks("b_id=") => serde_json::from_str::<serde_json::Value>(&renamed(recorded("brand_details"))).unwrap()["p"].to_string(),
        "/perfumes" if asks("y_0=") => recorded("rankings_year"),
        "/perfumes" => recorded("perfumes_text"),
        "/brand_details" => renamed(recorded("brand_details")),
        "/perfumer_details" => recorded("perfumer_details"),
        "/similar" => recorded("similar_list"),
        "/rating" => recorded("own_rating_none"),
        "/classification" => recorded("own_classification"),
        call if call.starts_with("/perfume_details/") => renamed(recorded("perfume_details")),
        _ => return json(StatusCode::NOT_FOUND, "{\"error\":\"Not found\"}".into()),
    };
    json(StatusCode::OK, answer)
}

async fn handler(State(parfumo): State<Parfumo>, method: Method, uri: Uri, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let text = |name: header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let target = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    let bearer = text(header::AUTHORIZATION).and_then(|v| v.strip_prefix("Bearer ").map(str::to_string));
    let asked = Asked { method: method.clone(), target: target.clone(), bearer: bearer.clone(), cookie: text(header::COOKIE).unwrap_or_default() };
    parfumo.asked.lock().unwrap().push(asked.clone());
    if asked.member_call() {
        return interface(&parfumo, asked.call().unwrap_or_default(), uri.query().unwrap_or_default(), bearer.as_deref());
    }
    if uri.path() == "/Perfumes/Dior" && parfumo.brand_on_the_web_only.load(Ordering::SeqCst) {
        return Html(fixture("brand.html")).into_response();
    }
    // Everything else is the web site of the account tests, asked as it was asked here.
    let mut forward = parfumo.http.request(method, format!("{}{target}", parfumo.upstream)).body(body);
    for name in [header::COOKIE, header::CONTENT_TYPE] {
        if let Some(value) = headers.get(&name) {
            forward = forward.header(name, value);
        }
    }
    let answer = forward.send().await.unwrap();
    let mut out = Response::builder().status(answer.status());
    for (name, value) in answer.headers() {
        if [header::SET_COOKIE, header::LOCATION, header::CONTENT_TYPE].contains(name) {
            out = out.header(name, value);
        }
    }
    out.body(Body::from(answer.bytes().await.unwrap())).unwrap()
}

/// A server whose Parfumo is the stand-in, with one user, `alice`, not linked yet.
async fn stand_in(user_min_interval: Duration) -> (TestApp, Parfumo, User) {
    let (upstream, site) = spawn_site().await;
    let http =
        reqwest::Client::builder().use_preconfigured_tls(parfumo_client::tls_config()).redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let parfumo = Parfumo { site, upstream, http, asked: Arc::default(), brand_on_the_web_only: Arc::default() };
    let router = Router::new().fallback(handler).with_state(parfumo.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let mut cfg = test_config(&base);
    cfg.user_min_interval = user_min_interval;
    let app = TestApp::with_config(cfg);
    let (id, _) = app.add_user("alice");
    (app, parfumo, User { id, name: "alice".into() })
}

/// The same, the account of `alice` linked with its e-mail and password: it holds a
/// session token of the application interface.
async fn linked_stand_in() -> (TestApp, Parfumo, User) {
    let (app, parfumo, user) = stand_in(Duration::from_millis(1)).await;
    link_account(&app.state.core, user.id, "alice").await;
    parfumo.forget_asked();
    (app, parfumo, user)
}

fn area<'a>(report: &'a Report, name: &str) -> Vec<&'a Check> {
    report.checks.iter().filter(|c| c.area == name).collect()
}

fn check<'a>(report: &'a Report, area: &str, name: &str) -> &'a Check {
    report.checks.iter().find(|c| c.area == area && c.name == name).unwrap_or_else(|| panic!("no check `{name}` in `{area}`:\n{}", report.text()))
}

#[tokio::test]
async fn the_interface_area_reads_the_application_interface_with_the_token_of_the_canary_account() {
    let (app, parfumo, user) = linked_stand_in().await;
    let token = parfumo.token_of("alice");
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["own nickname", "search", "perfume", "brand", "perfumer", "release year", "similar perfumes"], "{}", report.text());
    for c in &checks {
        assert_eq!(c.outcome, Outcome::Ok, "{}: {}", c.name, c.detail);
    }
    assert_eq!(check(&report, INTERFACE, "search").detail, "5 results");
    assert_eq!(check(&report, INTERFACE, "similar perfumes").detail, "4 similar perfumes");

    let asked = parfumo.asked();
    let calls: Vec<&Asked> = asked.iter().filter(|a| a.call().is_some()).collect();
    // Every read of the area was asked of the interface.
    for call in ["/auth/me", "/perfumes", "/perfume_details/9438", "/brand_details", "/perfumer_details", "/similar"] {
        assert!(calls.iter().any(|a| a.call() == Some(call)), "{call} was not asked: {calls:?}");
    }
    for query in ["filter=sauvage", "y_0=2015", "pf_id=509"] {
        assert!(calls.iter().any(|a| a.target.contains(query)), "{query} was not asked: {calls:?}");
    }
    // As the member, with the token of the account and none of its cookies; no login.
    assert!(calls.iter().all(|a| a.bearer.as_deref() == Some(token.as_str()) && a.cookie.is_empty()), "{calls:?}");
    // And of the interface alone: between the first call and the last, nothing was asked of
    // the web site, so no read of the area was left to it.
    let first = asked.iter().position(|a| a.call().is_some()).unwrap();
    let last = asked.iter().rposition(|a| a.call().is_some()).unwrap();
    assert!(asked[first..=last].iter().all(|a| a.call().is_some()), "a read of the area went to the web site: {:?}", &asked[first..=last]);
    // The public pages were read before, by a client without token nor cookie.
    assert!(asked[..first].iter().all(|a| a.bearer.is_none() && a.cookie.is_empty()), "{:?}", &asked[..first]);

    // The token is the account's, and the report is shown to the operator.
    let stored = serde_json::to_string(&report).unwrap() + &report.text();
    assert!(!stored.contains(&token) && !stored.contains("app-alice") && !stored.contains("pw-alice"), "the report must hold no secret");
    assert_eq!(parfumo.site.write_count(), 0, "the canary must never send a write");
}

#[tokio::test]
async fn the_three_web_panels_are_read_on_the_web_site_by_a_client_without_token() {
    let (app, parfumo, user) = linked_stand_in().await;
    let report = run(&app.state, Some(&user)).await;

    for name in ["own nickname", "rating panel", "classification panel"] {
        let c = check(&report, "account", name);
        assert_eq!(c.outcome, Outcome::Ok, "{name}: {}", c.detail);
    }
    let asked = parfumo.asked();
    // The two panels were asked of the web site, with the session of the account and no token.
    for panel in [RATE_PANEL, CLASSIFY_PANEL] {
        let sent: Vec<&Asked> = asked.iter().filter(|a| a.path() == panel).collect();
        assert!(!sent.is_empty(), "{panel} was not asked");
        assert!(sent.iter().all(|a| a.method == Method::POST && a.cookie.contains("pf_auth=") && a.bearer.is_none()), "{sent:?}");
    }
    // The interface has an answer for both, which the account client, holding a token,
    // would have taken: it was not asked.
    assert!(!asked.iter().any(|a| matches!(a.call(), Some("/rating" | "/classification"))), "a panel was read through the interface");
    // The nickname was read on the web page: the interface names another member here, and
    // the member's own page is asked under the name of the web page.
    assert!(asked.iter().any(|a| a.path() == "/Users/alice"), "the profile of the web nickname was not asked");
    assert!(!asked.iter().any(|a| a.path().starts_with("/Users/testuser")), "the nickname came from the interface");
    // The account itself keeps its token: the canary only set it aside in a copy.
    assert!(app.state.core.sessions.client_for(user.id).unwrap().has_app_session());
}

#[tokio::test]
async fn a_refused_token_is_one_failed_check_and_is_not_sent_again() {
    let (app, parfumo, user) = linked_stand_in().await;
    let stale = parfumo.token_of("alice");
    parfumo.end_app_sessions();
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    assert_eq!(checks.len(), 7, "{}", report.text());
    assert_eq!((checks[0].name.as_str(), checks[0].outcome), ("own nickname", Outcome::Failed), "{}", report.text());
    assert!(checks[0].detail.contains("refused the session token"), "{}", checks[0].detail);
    for c in &checks[1..] {
        assert_eq!(c.outcome, Outcome::Skipped, "{}: {}", c.name, c.detail);
    }
    assert!(checks[1].detail.contains("refused the session token at `own nickname`"), "{}", checks[1].detail);
    assert!(report.problems().iter().any(|c| c.area == INTERFACE), "a refusal is in the alert");
    assert!(!(serde_json::to_string(&report).unwrap() + &report.text()).contains(&stale), "the report must not name the token");

    let asked = parfumo.asked();
    let with_stale: Vec<&Asked> = asked.iter().filter(|a| a.bearer.as_deref() == Some(stale.as_str())).collect();
    assert_eq!(with_stale.len(), 1, "the refused token was sent again: {with_stale:?}");
    assert_eq!(with_stale[0].call(), Some("/auth/me"));
    // Nothing more was asked of the interface as a member, with any token or without one.
    assert_eq!(asked.iter().filter(|a| a.member_call()).count(), 1, "{:?}", asked.iter().filter(|a| a.call().is_some()).collect::<Vec<_>>());
    // The account forgot the refused token as well: its next call does not carry it.
    let _ = app.state.core.sessions.client_for(user.id).unwrap().app_nick().await;
    assert_eq!(parfumo.asked().iter().filter(|a| a.bearer.as_deref() == Some(stale.as_str())).count(), 1);
    // The rest of the run went on: the account is read on the web site.
    assert_eq!(check(&report, "account", "own nickname").outcome, Outcome::Ok);
}

#[tokio::test]
async fn without_a_token_the_interface_area_is_skipped_and_nothing_is_asked_of_it() {
    let (app, parfumo, user) = stand_in(Duration::from_millis(1)).await;
    let sessions = &app.state.core.sessions;
    // An account linked by the cookie of a browser: no e-mail nor password, so no token.
    let browser = sessions.uncached_public_client(user.id).unwrap();
    browser.login("alice", "pw-alice").await.unwrap();
    let cookie = browser.export_cookies().into_iter().find(|c| c.name == "pf_auth").unwrap();
    assert!(sessions.link_cookie(user.id, &format!("pf_auth={}", cookie.value)).await.unwrap());
    parfumo.forget_asked();

    let report = run(&app.state, Some(&user)).await;
    let checks = area(&report, INTERFACE);
    assert_eq!(checks.len(), 1, "{}", report.text());
    assert_eq!((checks[0].name.as_str(), checks[0].outcome), ("application session", Outcome::Skipped));
    assert!(checks[0].detail.contains("no session token of the application interface is held"), "{}", checks[0].detail);
    assert!(checks[0].detail.contains("nothing is asked of /app/v1"), "{}", checks[0].detail);
    let calls: Vec<Asked> = parfumo.asked().into_iter().filter(|a| a.call().is_some()).collect();
    assert!(calls.is_empty(), "the interface was asked without a token: {calls:?}");
    // The web panels are still read, on the web site.
    for name in ["own nickname", "rating panel", "classification panel"] {
        let c = check(&report, "account", name);
        assert_eq!(c.outcome, Outcome::Ok, "{name}: {}", c.detail);
    }

    // Without a canary account there is no such area at all: nothing to skip, nothing to alert about.
    let public_only = run(&app.state, None).await;
    assert!(public_only.checks.iter().all(|c| c.area == "public"), "{}", public_only.text());
}

#[tokio::test]
async fn an_answer_of_the_web_site_does_not_pass_for_one_of_the_interface() {
    let (app, parfumo, user) = linked_stand_in().await;
    parfumo.brand_on_the_web_only.store(true, Ordering::SeqCst);
    let report = run(&app.state, Some(&user)).await;

    // The interface does not know the brand: the client read its web page, and read it well.
    assert_eq!(check(&report, "public", "brand").outcome, Outcome::Ok, "{}", report.text());
    let asked = parfumo.asked();
    let brand_pages = asked.iter().filter(|a| a.path() == "/Perfumes/Dior").count();
    assert_eq!(brand_pages, 2, "once for the public area, once in place of the interface");
    // The canary says so: this is not what a linked user is served by the interface.
    let brand = check(&report, INTERFACE, "brand");
    assert_eq!(brand.outcome, Outcome::Changed, "{}", brand.detail);
    assert!(brand.detail.contains("the application interface did not answer this read"), "{}", brand.detail);
    // The token was not refused: the reads that do not need the brand are still made.
    assert_eq!(check(&report, INTERFACE, "release year").outcome, Outcome::Ok);
    assert_eq!(report.status, "changed");
}

#[tokio::test]
async fn a_run_that_was_stopped_asks_nothing_of_the_interface() {
    let (app, parfumo, user) = linked_stand_in().await;
    let mut r = Runner { stop: Some("run stopped after `search`: rate limited".into()), ..Runner::default() };
    interface_area(&mut r, &app.state.core.sessions, user.id).await;
    assert_eq!(r.checks.len(), 1);
    assert_eq!((r.checks[0].area.as_str(), r.checks[0].outcome), (INTERFACE, Outcome::Skipped));
    assert!(r.checks[0].detail.contains("run stopped after `search`"));
    assert!(parfumo.asked().is_empty(), "{:?}", parfumo.asked());
}

/// The two clients the canary makes for itself (the one that is lent the token, the copy of
/// the account session without it) take their turn in the request slot of the user.
#[tokio::test]
async fn the_clients_of_the_canary_wait_in_the_request_slot_of_the_user() {
    let interval = Duration::from_millis(100);
    let (app, parfumo, user) = stand_in(interval).await;
    link_account(&app.state.core, user.id, "alice").await;
    let sessions = &app.state.core.sessions;
    let account = sessions.account_client(user.id).await.unwrap();
    parfumo.forget_asked();

    let started = Instant::now();
    // A request of the account client takes the slot: every request after it has to wait.
    assert!(account.is_logged_in().await.unwrap());
    let mut r = Runner::default();
    interface_area(&mut r, sessions, user.id).await;
    let web = without_token(sessions, user.id, &account).unwrap();
    web.own_nick().await.unwrap();
    let elapsed = started.elapsed();

    assert!(r.checks.iter().all(|c| c.outcome == Outcome::Ok), "{:?}", r.checks);
    let requests = parfumo.asked().len() as u32;
    assert!(requests >= 10, "{requests} requests");
    // One interval before each request but the first. A client with a slot of its own
    // would send its first request at once, and the run would be one interval shorter.
    assert!(elapsed >= interval * (requests - 1), "{requests} requests in {elapsed:?}");
}

#[test]
fn a_listing_without_perfume_numbers_was_not_read_from_the_interface() {
    let row = |p_id| PerfumeSummary {
        id: "Dior/Sauvage".into(),
        p_id,
        name: "Sauvage".into(),
        url: "https://www.parfumo.com/Perfumes/Dior/Sauvage".into(),
        ..PerfumeSummary::default()
    };
    let listing = |p_id, total_items| Listing { items: vec![row(Some(1)), row(p_id)], page: 1, total_pages: Some(1), total_items };
    assert_eq!(index_rows(&listing(Some(2), Some(2)), "results"), Ok("2 results".into()));
    // The rows of a web page have no number.
    let web = index_rows(&listing(None, Some(2)), "results").unwrap_err();
    assert!(web.contains("no perfume number") && web.contains("the application interface did not answer this read"), "{web}");
    // The interface renamed its count: the tools could not page any more.
    assert_eq!(index_rows(&listing(Some(2), Some(0)), "results"), Err("the count of matches is missing".into()));
    assert_eq!(index_rows(&listing(Some(2), None), "results"), Err("the count of matches is missing".into()));
    let empty = Listing::<PerfumeSummary> { items: Vec::new(), page: 1, total_pages: Some(1), total_items: Some(3) };
    assert_eq!(index_rows(&empty, "results"), Err("no results found".into()));
}
