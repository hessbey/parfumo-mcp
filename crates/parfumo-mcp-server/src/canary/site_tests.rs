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
use parfumo_client::SimilarPerfume;
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
const LOGIN: &str = "/auth/login";

/// One request received by the stand-in.
#[derive(Clone, Debug)]
struct Asked {
    method: Method,
    /// Path and query.
    target: String,
    /// The token of the `Authorization: Bearer` header, if any.
    bearer: Option<String>,
    cookie: String,
    /// When it arrived, and the status it was answered with.
    at: Instant,
    status: u16,
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
        self.call().is_some_and(|call| call != LOGIN)
    }

    /// A call made as a member that the interface refused for its token.
    fn refused(&self) -> bool {
        self.member_call() && self.status == 401
    }
}

/// What a test makes happen once the stand-in has answered a call made as a member.
type AfterMemberCall = Box<dyn FnMut(&Asked) + Send>;

/// What a page that completes a read of the interface answers in place of its recorded page.
#[derive(Clone, Copy)]
struct PageTrouble {
    page: &'static str,
    status: StatusCode,
    body: &'static str,
}

/// A page with which the web site asks for a human. Not recorded: the title is the one the
/// client knows a challenge of Cloudflare by.
const CHALLENGE: &str = "<html><head><title>Just a moment...</title></head></html>";

/// The search of the `search` check, the second read of the area, and the listing of the
/// `release year` check: each is the only call of its check.
const SEARCH_CALL: &str = "filter=sauvage";
const YEAR_CALL: &str = "y_0=2015";

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
    /// The account has a rating of zero and a classification that is closed: the reader of
    /// the interface leaves both to the panels of the web site.
    own_reads_for_the_web_panels: Arc<AtomicBool>,
    after_member_call: Arc<Mutex<Option<AfterMemberCall>>>,
    /// What one completing page answers once a call was made as a member: the `public`
    /// area, which reads the same pages before, gets the recorded ones.
    page_trouble: Arc<Mutex<Option<PageTrouble>>>,
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

    fn after_member_call(&self, hook: impl FnMut(&Asked) + Send + 'static) {
        *self.after_member_call.lock().unwrap() = Some(Box::new(hook));
    }

    /// The site ends every session of the application interface each time it has answered
    /// one of these calls made as a member.
    fn end_app_sessions_after(&self, calls: &'static [&'static str]) {
        let site = self.site.clone();
        self.after_member_call(move |asked| {
            if asked.status == 200 && calls.iter().any(|call| asked.target.contains(call)) {
                site.app_live.lock().unwrap().clear();
            }
        });
    }

    fn page_trouble(&self, page: &'static str, status: StatusCode, body: &'static str) {
        *self.page_trouble.lock().unwrap() = Some(PageTrouble { page, status, body });
    }

    /// What a completing page answers now in place of its recorded page, if anything.
    fn trouble_of(&self, page: &str) -> Option<PageTrouble> {
        let trouble = self.page_trouble.lock().unwrap().filter(|trouble| trouble.page == page)?;
        self.asked.lock().unwrap().iter().any(Asked::member_call).then_some(trouble)
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

/// The recorded record of a perfume, under the number asked. Under its own number it is the
/// perfume the canary asks for; under any other (the perfumes similar to it) it is a perfume
/// with an address of its own, as on the real site: a record is about the number asked.
fn record_of(number: &str) -> String {
    let mut record: serde_json::Value = serde_json::from_str(&renamed(recorded("perfume_details"))).unwrap();
    if record["p"]["id"] != number {
        record["p"]["id"] = number.into();
        record["p"]["name_url"] = format!("perfume-{number}").into();
        record["b"]["name_url"] = format!("brand-{number}").into();
    }
    record.to_string()
}

/// A recorded answer with one value changed, and nothing else.
fn with_value(answer: String, recorded: &str, value: &str) -> String {
    assert_eq!(answer.matches(recorded).count(), 1, "`{recorded}` is not once in the recorded answer");
    answer.replace(recorded, value)
}

/// The application interface, with the recorded answers. Like the real one, it answers
/// nothing without a live token.
fn interface(parfumo: &Parfumo, call: &str, query: &str, bearer: Option<&str>) -> Response {
    let json = |status: StatusCode, text: String| (status, [(header::CONTENT_TYPE, "application/json")], text).into_response();
    if !bearer.is_some_and(|token| parfumo.site.app_live.lock().unwrap().contains_key(token)) {
        return json(StatusCode::UNAUTHORIZED, "{\"status\":\"error\",\"message\":\"Authorization header required\"}".into());
    }
    let asks = |param: &str| query.split('&').any(|pair| pair.starts_with(param));
    let for_the_web_panels = parfumo.own_reads_for_the_web_panels.load(Ordering::SeqCst);
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
        // No answer of an account with a rating of zero or a closed classification was
        // recorded: the recorded ones, with that one value.
        "/rating" if for_the_web_panels => with_value(recorded("own_rating"), "\"scent\": \"80\"", "\"scent\": \"0\""),
        "/classification" if for_the_web_panels => with_value(recorded("own_classification"), "\"locked\": 0", "\"locked\": 1"),
        "/rating" => recorded("own_rating_none"),
        "/classification" => recorded("own_classification"),
        call if call.starts_with("/perfume_details/") => record_of(call.trim_start_matches("/perfume_details/")),
        _ => return json(StatusCode::NOT_FOUND, "{\"error\":\"Not found\"}".into()),
    };
    json(StatusCode::OK, answer)
}

/// The three pages of the web site that complete a read of the interface: the page of the
/// perfume (its rank label), of its brand and of its perfumer (their country). The web site
/// of the account tests has none of them.
const COMPLETING_PAGES: [(&str, &str); 3] = [
    ("/Perfumes/Dior/Sauvage_Eau_de_Toilette", "perfume_sauvage.html"),
    ("/Perfumes/Dior", "brand.html"),
    ("/Perfumers/JeanChristophe_Herault", "perfumer.html"),
];

fn completing_page(path: &str) -> Option<&'static str> {
    COMPLETING_PAGES.iter().find(|(page, _)| *page == path).map(|(_, file)| *file)
}

/// Everything but the calls made as a member is the web site of the account tests, asked
/// as it was asked here.
async fn web_site(parfumo: &Parfumo, method: Method, target: &str, headers: &HeaderMap, body: axum::body::Bytes) -> Response {
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

async fn handler(State(parfumo): State<Parfumo>, method: Method, uri: Uri, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let text = |name: header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let target = uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    let bearer = text(header::AUTHORIZATION).and_then(|v| v.strip_prefix("Bearer ").map(str::to_string));
    let mut asked = Asked {
        method: method.clone(),
        target: target.clone(),
        bearer,
        cookie: text(header::COOKIE).unwrap_or_default(),
        at: Instant::now(),
        status: 0,
    };
    let answer = if asked.member_call() {
        interface(&parfumo, asked.call().unwrap_or_default(), uri.query().unwrap_or_default(), asked.bearer.as_deref())
    } else if let Some(page) = completing_page(uri.path()) {
        match parfumo.trouble_of(uri.path()) {
            Some(trouble) => (trouble.status, Html(trouble.body)).into_response(),
            None => Html(fixture(page)).into_response(),
        }
    } else {
        web_site(&parfumo, method, &target, &headers, body).await
    };
    asked.status = answer.status().as_u16();
    parfumo.asked.lock().unwrap().push(asked.clone());
    if asked.member_call()
        && let Some(hook) = parfumo.after_member_call.lock().unwrap().as_mut()
    {
        hook(&asked);
    }
    answer
}

/// A server whose Parfumo is the stand-in, with one user, `alice`, not linked yet.
async fn stand_in(user_min_interval: Duration) -> (TestApp, Parfumo, User) {
    let (upstream, site) = spawn_site().await;
    let http =
        reqwest::Client::builder().use_preconfigured_tls(parfumo_client::tls_config()).redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let parfumo = Parfumo {
        site,
        upstream,
        http,
        asked: Arc::default(),
        brand_on_the_web_only: Arc::default(),
        own_reads_for_the_web_panels: Arc::default(),
        after_member_call: Arc::default(),
        page_trouble: Arc::default(),
    };
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

/// The same, the account of `alice` linked by the cookie of a browser: no e-mail nor
/// password is held, so no token either.
async fn cookie_linked_stand_in() -> (TestApp, Parfumo, User) {
    let (app, parfumo, user) = stand_in(Duration::from_millis(1)).await;
    let sessions = &app.state.core.sessions;
    let browser = sessions.uncached_public_client(user.id).unwrap();
    browser.login("alice", "pw-alice").await.unwrap();
    let cookie = browser.export_cookies().into_iter().find(|c| c.name == "pf_auth").unwrap();
    assert!(sessions.link_cookie(user.id, &format!("pf_auth={}", cookie.value)).await.unwrap());
    parfumo.forget_asked();
    (app, parfumo, user)
}

fn area<'a>(report: &'a Report, name: &str) -> Vec<&'a Check> {
    report.checks.iter().filter(|c| c.area == name).collect()
}

fn check<'a>(report: &'a Report, area: &str, name: &str) -> &'a Check {
    report.checks.iter().find(|c| c.area == area && c.name == name).unwrap_or_else(|| panic!("no check `{name}` in `{area}`:\n{}", report.text()))
}

/// The verdict a run would have on the checks of the `interface` area alone. The stand-in is
/// the web site of the account tests, which has few of the public pages and not every page
/// of an account: the verdict of the whole run is `changed` there, whatever this area says.
fn interface_verdict(report: &Report) -> &'static str {
    Runner { checks: report.checks.iter().filter(|c| c.area == INTERFACE).cloned().collect(), ..Runner::default() }.status()
}

fn outcomes<'a>(checks: &[&'a Check]) -> Vec<(&'a str, Outcome)> {
    checks.iter().map(|c| (c.name.as_str(), c.outcome)).collect()
}

/// The checks of the `interface` area, in order.
const NAMES: [&str; 9] =
    ["own nickname", "search", "perfume", "brand", "perfumer", "release year", "similar perfumes", "own rating", "own classification"];

#[tokio::test]
async fn the_interface_area_reads_the_application_interface_with_the_token_of_the_canary_account() {
    let (app, parfumo, user) = linked_stand_in().await;
    let token = parfumo.token_of("alice");
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, NAMES, "{}", report.text());
    for c in &checks {
        assert_eq!(c.outcome, Outcome::Ok, "{}: {}", c.name, c.detail);
    }
    assert_eq!(check(&report, INTERFACE, "search").detail, "5 results");
    assert_eq!(check(&report, INTERFACE, "similar perfumes").detail, "4 similar perfumes");
    assert_eq!(check(&report, INTERFACE, "own rating").detail, "not rated");
    assert_eq!(check(&report, INTERFACE, "own classification").detail, "35 choices");
    assert_eq!(interface_verdict(&report), "ok", "{}", report.text());

    let asked = parfumo.asked();
    let calls: Vec<&Asked> = asked.iter().filter(|a| a.call().is_some()).collect();
    // Every read of the area was asked of the interface.
    for call in ["/auth/me", "/perfumes", "/perfume_details/9438", "/brand_details", "/perfumer_details", "/similar", "/rating", "/classification"] {
        assert!(calls.iter().any(|a| a.call() == Some(call)), "{call} was not asked: {calls:?}");
    }
    for query in ["filter=sauvage", "y_0=2015", "pf_id=509"] {
        assert!(calls.iter().any(|a| a.target.contains(query)), "{query} was not asked: {calls:?}");
    }
    // The member's own reads are those of the perfume the interface numbered.
    for call in ["/rating", "/classification"] {
        let sent: Vec<&&Asked> = calls.iter().filter(|a| a.call() == Some(call)).collect();
        assert!(sent.len() == 1 && sent[0].target.contains("p_id=9438"), "{sent:?}");
    }
    // As the member, with the token of the account and none of its cookies; no login.
    assert!(calls.iter().all(|a| a.bearer.as_deref() == Some(token.as_str()) && a.cookie.is_empty() && a.status == 200), "{calls:?}");
    // And of the interface: between the first call and the last, the web site was only
    // asked for the three pages that complete a read of the interface (the rank label of
    // the perfume, the country of the brand and of the perfumer), without token nor cookie.
    // No read of the area was left to it.
    let first = asked.iter().position(|a| a.call().is_some()).unwrap();
    let last = asked.iter().rposition(|a| a.call().is_some()).unwrap();
    let web: Vec<&Asked> = asked[first..=last].iter().filter(|a| a.call().is_none()).collect();
    assert_eq!(web.iter().map(|a| a.path()).collect::<Vec<_>>(), COMPLETING_PAGES.map(|(page, _)| page), "{web:?}");
    assert!(web.iter().all(|a| a.method == Method::GET && a.bearer.is_none() && a.cookie.is_empty()), "{web:?}");
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
    // would have taken. It was asked for each once, by the `interface` area: from the first
    // request made with the session of the account on, nothing is asked of it.
    let account_area = asked.iter().position(|a| a.cookie.contains("pf_auth=")).unwrap();
    assert!(!asked[account_area..].iter().any(|a| a.call().is_some()), "the account area read through the interface");
    for call in ["/rating", "/classification"] {
        assert_eq!(asked.iter().filter(|a| a.call() == Some(call)).count(), 1, "{call}");
    }
    // The nickname was read on the web page: the interface names another member here, and
    // the member's own page is asked under the name of the web page.
    assert!(asked.iter().any(|a| a.path() == "/Users/alice"), "the profile of the web nickname was not asked");
    assert!(!asked.iter().any(|a| a.path().starts_with("/Users/testuser")), "the nickname came from the interface");
    // The account itself keeps its token: the canary only set it aside in a copy.
    assert!(app.state.core.sessions.client_for(user.id).unwrap().has_app_session());
}

/// The token of the account did not live until this run. The refusal costs one check; the
/// sessions log in again as they do for the next tool call, and the area goes on.
#[tokio::test]
async fn a_refused_token_is_one_failed_check_and_the_area_goes_on_with_the_token_obtained_after_it() {
    let (app, parfumo, user) = linked_stand_in().await;
    let stale = parfumo.token_of("alice");
    parfumo.end_app_sessions();
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    assert_eq!(checks.len(), NAMES.len(), "{}", report.text());
    assert_eq!((checks[0].name.as_str(), checks[0].outcome), ("own nickname", Outcome::Failed), "{}", report.text());
    assert!(checks[0].detail.contains("refused the session token"), "{}", checks[0].detail);
    for c in &checks[1..] {
        assert_eq!(c.outcome, Outcome::Ok, "{}: {}", c.name, c.detail);
    }
    assert_eq!(report.problems().iter().filter(|c| c.area == INTERFACE).count(), 1, "the refusal is in the alert");
    assert_eq!(interface_verdict(&report), "inconclusive", "a refusal is not a changed structure, and not nothing");
    let fresh = parfumo.token_of("alice");
    let stored = serde_json::to_string(&report).unwrap() + &report.text();
    assert!(!stored.contains(&stale) && !stored.contains(&fresh), "the report must not name a token");

    let asked = parfumo.asked();
    // The refused token was sent once, and it is the only request the interface refused.
    let with_stale: Vec<&Asked> = asked.iter().filter(|a| a.bearer.as_deref() == Some(stale.as_str())).collect();
    assert_eq!(with_stale.len(), 1, "the refused token was sent again: {with_stale:?}");
    assert_eq!(with_stale[0].call(), Some("/auth/me"));
    assert_eq!(asked.iter().filter(|a| a.refused()).count(), 1);
    // One login, by the sessions; every call after it carries the token it gave.
    let logins: Vec<usize> = asked.iter().enumerate().filter(|(_, a)| a.call() == Some(LOGIN)).map(|(i, _)| i).collect();
    assert_eq!(logins.len(), 1, "{:?}", asked.iter().filter(|a| a.call().is_some()).collect::<Vec<_>>());
    let after: Vec<&Asked> = asked[logins[0]..].iter().filter(|a| a.member_call()).collect();
    assert!(after.len() >= 8 && after.iter().all(|a| a.bearer.as_deref() == Some(fresh.as_str())), "{after:?}");
    // The check that failed is not made again.
    assert_eq!(asked.iter().filter(|a| a.call() == Some("/auth/me")).count(), 1);
    // The rest of the run went on: the account is read on the web site.
    assert_eq!(check(&report, "account", "own nickname").outcome, Outcome::Ok);
}

/// The refused token is forgotten by the account as well, and no other is obtained: nothing
/// more is asked of the interface, with any token or without one.
#[tokio::test]
async fn after_a_refusal_nothing_is_asked_of_the_interface_without_a_new_token() {
    let (app, parfumo, user) = linked_stand_in().await;
    let stale = parfumo.token_of("alice");
    parfumo.end_app_sessions();
    parfumo.site.app_login_refused.store(true, Ordering::SeqCst);
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    assert_eq!(checks.len(), NAMES.len(), "{}", report.text());
    assert_eq!((checks[0].name.as_str(), checks[0].outcome), ("own nickname", Outcome::Failed), "{}", report.text());
    for c in &checks[1..] {
        assert_eq!(c.outcome, Outcome::Skipped, "{}: {}", c.name, c.detail);
        // The page of the perfumer has a reason of its own: the record that names it.
        let why = if c.name == "perfumer" { "needs the perfumer" } else { "at `own nickname`, and the account obtained no other" };
        assert!(c.detail.contains(why), "{}: {}", c.name, c.detail);
    }
    assert!(report.problems().iter().any(|c| c.area == INTERFACE), "a refusal is in the alert");

    let asked = parfumo.asked();
    assert_eq!(asked.iter().filter(|a| a.member_call()).count(), 1, "{:?}", asked.iter().filter(|a| a.call().is_some()).collect::<Vec<_>>());
    // The sessions asked for a new token once, for the whole run: not once per check, and
    // not again for the account area.
    assert_eq!(asked.iter().filter(|a| a.call() == Some(LOGIN)).count(), 1);
    // The account forgot the refused token as well: its next call does not carry it.
    let _ = app.state.core.sessions.client_for(user.id).unwrap().app_nick().await;
    assert_eq!(parfumo.asked().iter().filter(|a| a.bearer.as_deref() == Some(stale.as_str())).count(), 1);
    assert_eq!(check(&report, "account", "own nickname").outcome, Outcome::Ok);
}

/// A refusal in the middle of the area, at a read the web site can answer: the client reads
/// the web page, and reads it well. That is not an answer of the interface.
#[tokio::test]
async fn a_token_refused_at_a_later_check_is_not_hidden_by_the_answer_of_the_web_site() {
    let (app, parfumo, user) = linked_stand_in().await;
    let stale = parfumo.token_of("alice");
    // The nickname and the search are answered; the token is dead for the record.
    parfumo.end_app_sessions_after(&[SEARCH_CALL]);
    parfumo.site.app_login_refused.store(true, Ordering::SeqCst);
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    let mut expected = vec![("own nickname", Outcome::Ok), ("search", Outcome::Ok), ("perfume", Outcome::Failed)];
    expected.extend(NAMES[3..].iter().map(|name| (*name, Outcome::Skipped)));
    assert_eq!(outcomes(&checks), expected, "{}", report.text());
    let perfume = check(&report, INTERFACE, "perfume");
    assert!(perfume.detail.contains("refused the session token"), "{}", perfume.detail);
    assert!(checks[3].detail.contains("refused the session token at `perfume`"), "{}", checks[3].detail);

    let asked = parfumo.asked();
    let refused = asked.iter().position(|a| a.refused()).unwrap();
    // The web site answered the read in place of the interface.
    assert!(asked[refused..].iter().any(|a| a.path() == "/Perfumes/Dior/Sauvage_Eau_de_Toilette" && a.status == 200), "{:?}", &asked[refused..]);
    // No call as a member follows the refusal, and the refused token was sent that once.
    assert!(!asked[refused + 1..].iter().any(|a| a.member_call()), "{:?}", &asked[refused + 1..]);
    assert_eq!(asked.iter().filter(|a| a.refused()).count(), 1);
    assert_eq!(asked[refused].bearer.as_deref(), Some(stale.as_str()));
    assert_eq!(asked.iter().filter(|a| a.call() == Some(LOGIN)).count(), 1);
}

/// The site refuses the token of the account, then the one obtained after it: the area
/// stops there. Two refused requests are what the site answers by blocking the address.
#[tokio::test]
async fn a_second_refused_token_ends_the_area() {
    let (app, parfumo, user) = linked_stand_in().await;
    // The token of the account dies after the search; the next one after the release year.
    parfumo.end_app_sessions_after(&[SEARCH_CALL, YEAR_CALL]);
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    let expected = [
        ("own nickname", Outcome::Ok),
        ("search", Outcome::Ok),
        ("perfume", Outcome::Failed),
        ("brand", Outcome::Ok),
        // Needs the record of the perfume.
        ("perfumer", Outcome::Skipped),
        ("release year", Outcome::Ok),
        ("similar perfumes", Outcome::Failed),
        ("own rating", Outcome::Skipped),
        ("own classification", Outcome::Skipped),
    ];
    assert_eq!(outcomes(&checks), expected, "{}", report.text());
    let stopped = check(&report, INTERFACE, "own rating");
    assert!(stopped.detail.contains("at `perfume`, then the one obtained after it at `similar perfumes`"), "{}", stopped.detail);

    let asked = parfumo.asked();
    let refused: Vec<usize> = asked.iter().enumerate().filter(|(_, a)| a.refused()).map(|(i, _)| i).collect();
    assert_eq!(refused.len(), 2, "{:?}", asked.iter().filter(|a| a.call().is_some()).collect::<Vec<_>>());
    // Two tokens, each refused once.
    assert_ne!(asked[refused[0]].bearer, asked[refused[1]].bearer);
    // One login between the two refusals, and no call as a member after the second: the
    // token the account obtains for itself later in the run is not used by the canary.
    assert_eq!(asked[refused[0]..refused[1]].iter().filter(|a| a.call() == Some(LOGIN)).count(), 1);
    assert!(!asked[refused[1] + 1..].iter().any(|a| a.member_call()), "{:?}", &asked[refused[1] + 1..]);
}

/// A tool call of the same user met the refusal of the token before the run: the token is
/// gone for every client of the user at once, the canary's included. The canary reads with
/// the token the sessions obtain next, and never sends the refused one.
#[tokio::test]
async fn a_token_refused_to_a_tool_call_is_not_sent_by_the_canary() {
    let (app, parfumo, user) = linked_stand_in().await;
    let sessions = &app.state.core.sessions;
    let stale = parfumo.token_of("alice");
    // What a tool call does: it reads as the member, and the site has ended the session.
    parfumo.end_app_sessions();
    let account = sessions.client_for(user.id).unwrap();
    assert!(account.app_nick().await.is_err());
    sessions.store_app_session(user.id);
    assert!(!account.has_app_session() && account.app_session_ended());

    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    assert_eq!(outcomes(&checks), NAMES.map(|name| (name, Outcome::Ok)), "{}", report.text());
    let asked = parfumo.asked();
    let fresh = parfumo.token_of("alice");
    assert_ne!(fresh, stale);
    let member: Vec<&Asked> = asked.iter().filter(|a| a.member_call()).collect();
    // The refused token left once, with the tool call; every call of the canary bears the new one.
    assert_eq!(member.iter().filter(|a| a.refused()).count(), 1, "{member:?}");
    assert!(member[0].refused() && member[0].bearer.as_deref() == Some(stale.as_str()), "{member:?}");
    assert!(member.len() > 1 && member[1..].iter().all(|a| a.bearer.as_deref() == Some(fresh.as_str())), "{member:?}");
    // The token the account obtained is still its own after the run.
    assert!(sessions.client_for(user.id).unwrap().has_app_session());
    assert_eq!(asked.iter().filter(|a| a.call() == Some(LOGIN)).count(), 1);
}

/// The interface answers the record of the perfume; the web page that completes it asks
/// for a human. The tools return the record with a warning. For the canary it is the
/// captcha of any web read: no verdict, and nothing more is sent.
#[tokio::test]
async fn a_captcha_on_a_completing_page_stops_the_run() {
    let (app, parfumo, user) = linked_stand_in().await;
    parfumo.page_trouble("/Perfumes/Dior/Sauvage_Eau_de_Toilette", StatusCode::FORBIDDEN, CHALLENGE);
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    let mut expected = vec![("own nickname", Outcome::Ok), ("search", Outcome::Ok), ("perfume", Outcome::Failed)];
    expected.extend(NAMES[3..].iter().map(|name| (*name, Outcome::Skipped)));
    assert_eq!(outcomes(&checks), expected, "{}", report.text());
    let perfume = check(&report, INTERFACE, "perfume");
    assert_eq!(perfume.detail, ClientError::CaptchaRequired.to_string());
    for c in checks[3..].iter().chain(&area(&report, "account")) {
        // The page of the perfumer has a reason of its own: the record that names it.
        let why = if c.name == "perfumer" { "needs the perfumer" } else { "run stopped after `perfume`: Parfumo requires a human verification" };
        assert!(c.detail.contains(why), "{}: {}", c.name, c.detail);
    }
    assert_eq!(area(&report, "account").len(), 1, "{}", report.text());
    assert_eq!(interface_verdict(&report), "inconclusive", "a captcha is not a changed structure");

    // The interface had answered; the challenge is the last thing the site was asked for.
    let asked = parfumo.asked();
    let last = asked.last().unwrap();
    assert_eq!((last.path(), last.status), ("/Perfumes/Dior/Sauvage_Eau_de_Toilette", 403), "{last:?}");
    assert!(asked.iter().any(|a| a.call() == Some("/perfume_details/9438") && a.status == 200));
    assert!(!asked.iter().any(|a| a.call() == Some("/brand_details")), "the next check was sent");
}

/// The web page that completes the brand cannot be reached. That is no verdict on the
/// structure of anything, and no reason to stop: the checks that follow are made.
///
/// The client tries the page again three times, two, four and eight seconds apart, as it
/// does for a tool; the server has no setting for it, so this test lasts as long.
#[tokio::test]
async fn an_outage_of_a_completing_page_is_a_failed_check_and_the_run_goes_on() {
    let (app, parfumo, user) = linked_stand_in().await;
    // Not recorded: any page under status 503 is an outage for the client.
    parfumo.page_trouble("/Perfumes/Dior", StatusCode::SERVICE_UNAVAILABLE, "<html><body>Service Unavailable</body></html>");
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    let expected: Vec<(&str, Outcome)> = NAMES.iter().map(|name| (*name, if *name == "brand" { Outcome::Failed } else { Outcome::Ok })).collect();
    assert_eq!(outcomes(&checks), expected, "{}", report.text());
    let brand = check(&report, INTERFACE, "brand");
    assert_eq!(brand.detail, "upstream error: HTTP 503 on /Perfumes/Dior");
    assert_eq!(interface_verdict(&report), "inconclusive", "an outage is not a changed structure");
    // The interface itself answered the brand, and the account is read as on any day.
    let asked = parfumo.asked();
    assert!(asked.iter().any(|a| a.call() == Some("/brand_details") && a.status == 200));
    assert_eq!(asked.iter().filter(|a| a.path() == "/Perfumes/Dior" && a.status == 503).count(), 4);
    assert_eq!(check(&report, "account", "own nickname").outcome, Outcome::Ok);
}

/// The web page that completes the perfumer is gone. That page did change: the answer of
/// the interface comes with the warning of the reader, and the check says so.
#[tokio::test]
async fn a_completing_page_that_is_gone_is_a_changed_page() {
    let (app, parfumo, user) = linked_stand_in().await;
    // Not recorded: any page under status 404 is a missing page for the client.
    parfumo.page_trouble("/Perfumers/JeanChristophe_Herault", StatusCode::NOT_FOUND, "<html><body>Not found</body></html>");
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    let expected: Vec<(&str, Outcome)> = NAMES.iter().map(|name| (*name, if *name == "perfumer" { Outcome::Changed } else { Outcome::Ok })).collect();
    assert_eq!(outcomes(&checks), expected, "{}", report.text());
    let perfumer = check(&report, INTERFACE, "perfumer");
    assert_eq!(perfumer.detail, "parsed, but the reader warns: country unavailable: not found: /Perfumers/JeanChristophe_Herault");
    assert_eq!(interface_verdict(&report), "changed");
}

#[tokio::test]
async fn an_account_linked_by_cookie_has_no_interface_area_and_that_is_no_trouble() {
    let (app, parfumo, user) = cookie_linked_stand_in().await;
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    assert_eq!(checks.len(), 1, "{}", report.text());
    assert_eq!((checks[0].name.as_str(), checks[0].outcome), ("application session", Outcome::NotApplicable));
    assert!(checks[0].detail.contains("linked by the cookie of a browser"), "{}", checks[0].detail);
    assert!(checks[0].detail.contains("nothing is asked of /app/v1"), "{}", checks[0].detail);
    let calls: Vec<Asked> = parfumo.asked().into_iter().filter(|a| a.call().is_some()).collect();
    assert!(calls.is_empty(), "the interface was asked without a token: {calls:?}");
    // The web panels are still read, on the web site.
    for name in ["own nickname", "rating panel", "classification panel"] {
        let c = check(&report, "account", name);
        assert_eq!(c.outcome, Outcome::Ok, "{name}: {}", c.detail);
    }
    // This is a sound setup, every day: no alert, and the line says what was not read.
    assert_eq!(interface_verdict(&report), "ok", "{}", report.text());
    assert!(!report.problems().iter().any(|c| c.area == INTERFACE), "{:?}", report.problems());
    assert!(report.summary().contains(" skipped, 1 not applicable ("), "{}", report.summary());
    assert!(report.text().contains("n/a      interface  application session"), "{}", report.text());

    // Without a canary account there is no such area at all: nothing to skip, nothing to alert about.
    let public_only = run(&app.state, None).await;
    assert!(public_only.checks.iter().all(|c| c.area == "public"), "{}", public_only.text());
}

/// The account is linked with its e-mail and password, and the interface gives it no token:
/// what every user linked this way then gets. That is not the lasting state of a sound setup.
#[tokio::test]
async fn an_account_linked_with_its_password_that_gets_no_token_is_a_failed_check() {
    let (app, parfumo, user) = stand_in(Duration::from_millis(1)).await;
    parfumo.site.app_login_refused.store(true, Ordering::SeqCst);
    link_account(&app.state.core, user.id, "alice").await;
    assert!(!app.state.core.sessions.client_for(user.id).unwrap().has_app_session());
    parfumo.forget_asked();

    let report = run(&app.state, Some(&user)).await;
    let checks = area(&report, INTERFACE);
    assert_eq!(checks.len(), 1, "{}", report.text());
    assert_eq!((checks[0].name.as_str(), checks[0].outcome), ("application session", Outcome::Failed));
    assert!(checks[0].detail.contains("its login to the interface gave none"), "{}", checks[0].detail);
    assert!(!checks[0].detail.contains("nothing is asked"), "a login was asked: {}", checks[0].detail);
    assert!(report.problems().iter().any(|c| c.area == INTERFACE), "it is in the alert");
    assert_eq!(interface_verdict(&report), "inconclusive", "{}", report.text());
    // Whether to try the login again is for the sessions to say, and never more than once
    // in a run; the canary asked nothing as a member.
    let calls: Vec<Asked> = parfumo.asked().into_iter().filter(|a| a.call().is_some()).collect();
    assert!(calls.len() <= 1 && calls.iter().all(|a| a.call() == Some(LOGIN)), "{calls:?}");
    assert!(!(serde_json::to_string(&report).unwrap() + &report.text()).contains("pw-alice"), "the report must hold no secret");
}

#[tokio::test]
async fn a_user_without_a_linked_account_has_the_area_skipped() {
    let (app, parfumo, user) = stand_in(Duration::from_millis(1)).await;
    let mut r = Runner::default();
    interface_area(&mut r, &app.state.core.sessions, user.id, None).await;
    assert_eq!(r.checks.len(), 1);
    assert_eq!((r.checks[0].name.as_str(), r.checks[0].outcome), ("application session", Outcome::Skipped));
    assert!(r.checks[0].detail.contains("no Parfumo account is linked"), "{}", r.checks[0].detail);
    assert_eq!(r.status(), "inconclusive");
    assert!(parfumo.asked().is_empty(), "{:?}", parfumo.asked());
}

/// The account has a rating of zero and a closed classification: the reader of the
/// interface leaves both reads to the panels of the web site, which answer nothing to a
/// client without web session. Nothing changed, and nothing is wrong with the setup.
#[tokio::test]
async fn an_own_read_the_interface_leaves_to_the_web_panel_does_not_apply() {
    let (app, parfumo, user) = linked_stand_in().await;
    parfumo.own_reads_for_the_web_panels.store(true, Ordering::SeqCst);
    let report = run(&app.state, Some(&user)).await;

    let checks = area(&report, INTERFACE);
    let mut expected: Vec<(&str, Outcome)> = NAMES[..7].iter().map(|name| (*name, Outcome::Ok)).collect();
    expected.extend([("own rating", Outcome::NotApplicable), ("own classification", Outcome::NotApplicable)]);
    assert_eq!(outcomes(&checks), expected, "{}", report.text());
    let rating = check(&report, INTERFACE, "own rating");
    assert!(rating.detail.contains("left this read to the panel of the web site, as it does for a rating of zero"), "{}", rating.detail);
    let classification = check(&report, INTERFACE, "own classification");
    assert!(classification.detail.contains("as it does when the account has a vote on this perfume"), "{}", classification.detail);
    assert_eq!(interface_verdict(&report), "ok", "{}", report.text());

    let asked = parfumo.asked();
    // The interface was asked, answered, and its token is still the account's.
    for call in ["/rating", "/classification"] {
        assert!(asked.iter().any(|a| a.call() == Some(call) && a.status == 200), "{call}");
    }
    assert!(!asked.iter().any(|a| a.refused()));
    assert!(app.state.core.sessions.client_for(user.id).unwrap().has_app_session());
    // The panels were then asked without the session of the account, which gave nothing;
    // the account area read them with it.
    for panel in [RATE_PANEL, CLASSIFY_PANEL] {
        assert!(asked.iter().any(|a| a.path() == panel && a.cookie.is_empty()), "{panel} was not left to the web site");
    }
    for name in ["rating panel", "classification panel"] {
        assert_eq!(check(&report, "account", name).outcome, Outcome::Ok);
    }
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

/// The record of a perfume as the web page gives it, read by a client without token, and as
/// the interface gives it: only the second bears the mark the check looks for.
#[tokio::test]
async fn the_record_of_the_web_page_does_not_pass_for_the_record_of_the_interface() {
    let (app, _parfumo, user) = linked_stand_in().await;
    let sessions = &app.state.core.sessions;
    let web = sessions.uncached_public_client(user.id).unwrap().get_perfume(PERFUME).await.unwrap();
    assert!(web.p_id.is_some() && !web.name.is_empty() && !web.brand.name.is_empty(), "{web:?}");
    let refused = interface_record(&web).unwrap_err();
    assert!(refused.contains("the brand has no number") && refused.contains(NOT_THE_INTERFACE), "{refused}");

    let client = sessions.uncached_app_client(user.id).unwrap();
    sessions.open_app_session(user.id).await;
    let record = client.get_perfume(PERFUME).await.unwrap();
    assert!(interface_record(&record).is_ok(), "{:?}", interface_record(&record));
    // The mark is that number and nothing else.
    let mut unnumbered = record.clone();
    unnumbered.brand.numeric_id = None;
    assert!(interface_record(&unnumbered).unwrap_err().contains(NOT_THE_INTERFACE));
    // A record of the interface that lost a part says so: unlike the web record, it does not warn.
    let mut bare = record;
    bare.classification.clear();
    assert!(interface_record(&bare).unwrap_err().contains("0 classification groups"));
}

#[test]
fn a_similar_list_with_the_votes_of_its_resolved_rows_only_was_not_read_from_the_interface() {
    let row = |votes| SimilarPerfume {
        p_id: 1,
        name: "Sauvage".into(),
        votes,
        url: Some("https://www.parfumo.com/Perfumes/Dior/Sauvage".into()),
        ..SimilarPerfume::default()
    };
    let list = |rows: Vec<SimilarPerfume>| SimilarPerfumes { perfume_id: PERFUME.into(), similar: rows, ..SimilarPerfumes::default() };
    // The interface sends the votes with every row.
    assert_eq!(interface_similar(&list(vec![row(Some(9)), row(Some(4)), row(Some(2))]), 1), Ok("3 similar perfumes".into()));
    // The web list has them for the rows whose link it asked for: the first, here.
    let web = interface_similar(&list(vec![row(Some(9)), row(None), row(None)]), 1).unwrap_err();
    assert!(web.contains("no row after the first has its votes") && web.contains(NOT_THE_INTERFACE), "{web}");
    // With two rows resolved, the votes of the second prove nothing.
    assert!(interface_similar(&list(vec![row(Some(9)), row(Some(4)), row(None)]), 2).is_err());
    assert!(interface_similar(&list(vec![row(Some(9)), row(Some(4)), row(Some(2))]), 2).is_ok());
    let mut warned = list(vec![row(Some(9)), row(Some(4))]);
    warned.warnings.push("layering unavailable".into());
    assert!(interface_similar(&warned, 1).unwrap_err().contains("the reader warns"));
}

#[tokio::test]
async fn a_run_that_was_stopped_asks_nothing_of_the_interface() {
    let (app, parfumo, user) = linked_stand_in().await;
    let mut r = Runner { stop: Some("run stopped after `search`: rate limited".into()), ..Runner::default() };
    interface_area(&mut r, &app.state.core.sessions, user.id, None).await;
    assert_eq!(r.checks.len(), 1);
    assert_eq!((r.checks[0].area.as_str(), r.checks[0].outcome), (INTERFACE, Outcome::Skipped));
    assert!(r.checks[0].detail.contains("run stopped after `search`"));
    assert!(parfumo.asked().is_empty(), "{:?}", parfumo.asked());
}

/// The two clients the canary makes for itself (the one that is lent the token, the copy of
/// the account session without it) take their turn in the request slot of the user.
#[tokio::test]
async fn the_clients_of_the_canary_wait_in_the_request_slot_of_the_user() {
    let interval = Duration::from_millis(200);
    let (app, parfumo, user) = stand_in(interval).await;
    link_account(&app.state.core, user.id, "alice").await;
    let sessions = &app.state.core.sessions;
    let account = sessions.account_client(user.id).await.unwrap();
    parfumo.forget_asked();

    // A request of the account client takes the slot: every request after it has to wait.
    assert!(account.is_logged_in().await.unwrap());
    let mut r = Runner::default();
    interface_area(&mut r, sessions, user.id, None).await;
    let web = without_token(sessions, user.id, &account).unwrap();
    web.own_nick().await.unwrap();

    assert!(r.checks.iter().all(|c| c.outcome == Outcome::Ok), "{:?}", r.checks);
    let asked = parfumo.asked();
    assert!(asked.len() >= 12, "{} requests", asked.len());
    // Each request left one interval after the one before, whichever client sent it: a
    // client with a slot of its own would send its first request at once. Arrivals are
    // compared, hence the margin; none of it is needed to tell an interval from no wait.
    for pair in asked.windows(2) {
        let gap = pair[1].at.duration_since(pair[0].at);
        assert!(gap >= interval / 2, "{} came {gap:?} after {}", pair[1].target, pair[0].target);
    }
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

#[test]
fn only_what_does_not_apply_to_the_account_leaves_the_verdict_alone() {
    let run_with = |outcome| {
        let mut r = Runner::default();
        r.push("public", "search", "search_perfumes", Outcome::Ok, "5 results".into(), None);
        r.push(INTERFACE, "application session", "open_app_session", outcome, String::new(), None);
        r.status()
    };
    assert_eq!(run_with(Outcome::NotApplicable), "ok");
    assert_eq!(run_with(Outcome::Skipped), "inconclusive");
    assert_eq!(run_with(Outcome::Failed), "inconclusive");
    assert_eq!(run_with(Outcome::Changed), "changed");

    let link = |mode: &str| LinkInfo { mode: mode.into(), status: "linked".into(), linked_at: 0, last_login_at: None };
    assert_eq!(without_session(Ok(Some(link("cookie")))).0, Outcome::NotApplicable);
    assert_eq!(without_session(Ok(Some(link("credentials")))).0, Outcome::Failed);
    assert_eq!(without_session(Ok(None)).0, Outcome::Skipped);
    assert_eq!(
        without_session(Err(SessionError::Internal("database is locked".into()))),
        (Outcome::Failed, "internal error: database is locked".into())
    );
}
