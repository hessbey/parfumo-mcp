//! Account side of the client against a local mock (no network): write semantics measured
//! at jalon 0 bis (DISCOVERY §3.9) and ShelfControl reads on the recorded fixtures.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Form, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use parfumo_client::{ClientConfig, ClientError, Expect, GlobalLimiter, ParfumoClient, UserLimiter, calls};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[derive(Default)]
struct Mock {
    alive: AtomicUsize,
    hits: Mutex<HashMap<String, usize>>,
    forms: Mutex<Vec<(String, HashMap<String, String>)>>,
}

impl Mock {
    fn count(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

type S = State<Arc<Mock>>;

async fn alive(State(m): S) -> Response {
    let n = m.alive.fetch_add(1, Ordering::SeqCst) + 1;
    format!("{{\"csrf_key\":\"mockkey00000000{n}\"}}").into_response()
}

/// The first key handed out is stale: `/api/*` answers 403, `/action/**` an empty 200.
async fn write(State(m): S, Path(path): Path<String>, Form(form): Form<HashMap<String, String>>) -> Response {
    let path = format!("/{path}");
    *m.hits.lock().unwrap().entry(path.clone()).or_default() += 1;
    let stale = form.get("csrf_key").map(String::as_str) != Some("mockkey000000002");
    m.forms.lock().unwrap().push((path.clone(), form));
    if stale {
        return if path.starts_with("/api/") {
            (StatusCode::FORBIDDEN, "{\"error\":\"Invalid CSRF token\"}").into_response()
        } else {
            "".into_response()
        };
    }
    match path.as_str() {
        "/api/wardrobe/wardrobe_add_remove" => "".into_response(),
        "/api/wardrobe/update_details" => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        "/api/memo/create_memo" => (StatusCode::UNAUTHORIZED, "{\"error\":\"Authentication required\"}").into_response(),
        "/action/shelfcontrol/columns.php" => "{\"ok\":true}".into_response(),
        "/api/wardrobe/wardrobe_copy_move" => "{\"error_title\":\"Move not possible\",\"error_message\":\"Already there\"}".into_response(),
        "/api/account/update_iwear_date" => "1".into_response(),
        "/api/wardrobe/add_list" => "{\"status\":\"ok\",\"id\":922018}".into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn shelf(State(m): S, path: Option<Path<String>>) -> Response {
    let slug = path.map(|Path(p)| p).unwrap_or_default();
    *m.hits.lock().unwrap().entry(format!("shelf:{slug}")).or_default() += 1;
    let page = fixture("auth_shelfcontrol.html");
    match slug.as_str() {
        "want" => (StatusCode::FOUND, [(header::LOCATION, "/action/dologin?redirect=x")]).into_response(),
        "922018" => page.replace("data-list=\"1\"", "data-list=\"922018\"").into_response(),
        // Like a site that would fall back to the default list for an unknown one.
        _ => page.into_response(),
    }
}

async fn csv() -> Response {
    fixture("auth_wardrobe_export.csv").into_response()
}

async fn start() -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    let app = Router::new()
        .route("/api/alive", get(alive))
        .route("/account/shelfcontrol", get(shelf))
        .route("/account/shelfcontrol/{slug}", get(shelf))
        .route("/action/wardrobe/generate_wardrobe_csv.php", get(csv))
        .route("/{*path}", post(write))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), mock)
}

fn config(base: &str) -> ClientConfig {
    ClientConfig {
        base_url: base.to_string(),
        min_interval: Duration::ZERO,
        jitter: Duration::ZERO,
        max_retries: 3,
        backoff_base: Duration::from_millis(5),
        timeout: Duration::from_secs(5),
        ..ClientConfig::default()
    }
}

fn client(base: &str) -> ParfumoClient {
    ParfumoClient::new(config(base), GlobalLimiter::new(0), None).unwrap()
}

#[tokio::test]
async fn writes_carry_the_csrf_key_and_replay_only_on_a_stale_key() {
    let (base, mock) = start().await;
    let c = client(&base);

    // /api: 403 Invalid CSRF token → key renewed, sent once more. An empty answer is the
    // normal success of this route and is not replayed.
    let add = calls::collection_add(63363, 1);
    assert_eq!(add.expect, Expect::Anything);
    assert_eq!(c.execute(&add).await.unwrap(), "");
    assert_eq!(mock.count("/api/wardrobe/wardrobe_add_remove"), 2, "one stale attempt, one accepted");
    assert_eq!(c.execute(&add).await.unwrap(), "");
    assert_eq!(mock.count("/api/wardrobe/wardrobe_add_remove"), 3, "an empty success is never replayed");
    let forms = mock.forms.lock().unwrap().clone();
    assert_eq!(forms.last().unwrap().1.get("p").map(String::as_str), Some("63363"));
    assert_eq!(forms.last().unwrap().1.get("action").map(String::as_str), Some("true"));
    assert!(!add.form.contains_key("csrf_key"), "the token is added at send time only");
    assert!(!serde_json::to_string(&add).unwrap().contains("mockkey"));
}

#[tokio::test]
async fn action_route_with_a_stale_key_answers_empty_and_is_replayed_once() {
    let (base, mock) = start().await;
    let c = client(&base);
    let body = c.execute(&calls::collection_remaining_ml(119519310, "60")).await.unwrap();
    assert_eq!(body, "{\"ok\":true}");
    assert_eq!(mock.count("/action/shelfcontrol/columns.php"), 2);
    assert_eq!(mock.alive.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn write_failures_are_never_retried_and_are_normalised() {
    let (base, mock) = start().await;
    let c = client(&base);
    // The first call renews the stale key; the following ones use the accepted key.
    assert_eq!(c.execute(&calls::wear_set_date("1", "2026-11-12T07:21")).await.unwrap(), "1");

    // 5xx: reads back off and retry, writes do not (a write must not be sent twice).
    let err = c.execute(&calls::collection_details(1, 1, "100", "85", "EUR")).await.unwrap_err();
    assert!(matches!(err, ClientError::Upstream(_)), "{err:?}");
    assert_eq!(mock.count("/api/wardrobe/update_details"), 1);

    // 401: the session is gone.
    assert_eq!(c.execute(&calls::collection_note(1, "hello")).await.unwrap_err(), ClientError::LoginRequired);

    // A refusal in the body is an error carrying the site's own words.
    match c.execute(&calls::collection_copy_move(1, 2, 1, 3, false)).await.unwrap_err() {
        ClientError::Upstream(msg) => assert!(msg.contains("Move not possible") && msg.contains("wardrobe_copy_move"), "{msg}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(mock.count("/api/wardrobe/wardrobe_copy_move"), 1);

    let body = c.execute(&calls::list_create()).await.unwrap();
    assert!(body.contains("922018"));
    // An empty note is a deletion, on another route.
    assert_eq!(calls::collection_note(7, "  ").path, "/api/memo/delete_memo");
    assert_eq!(calls::collection_remove(5, 1).form.get("action").map(String::as_str), Some("false"));
}

#[tokio::test]
async fn shelf_reads_resolve_lists_and_refuse_silent_fallbacks() {
    let (base, mock) = start().await;
    let c = client(&base);

    let have = c.shelf("I_have").await.unwrap();
    assert_eq!((have.list.id, have.items.len()), (1, 1));
    assert_eq!(mock.count("shelf:have"), 1, "a fixed list needs one request");

    // A custom list by name: default page to resolve it, then its own page.
    let custom = c.shelf("TestList").await.unwrap();
    assert_eq!(custom.list.id, 922018);
    assert_eq!((mock.count("shelf:"), mock.count("shelf:922018")), (1, 1));

    // The site answering with another list than the one asked is not accepted.
    assert!(matches!(c.shelf("555555").await.unwrap_err(), ClientError::NotFound(_)));
    assert!(matches!(c.shelf("my imaginary list").await.unwrap_err(), ClientError::NotFound(_)));
    // Redirect to the login page: the session expired.
    assert_eq!(c.shelf("want").await.unwrap_err(), ClientError::LoginRequired);

    let csv = c.collection_csv(1).await.unwrap();
    assert!(csv.starts_with("\"Perfume\",\"Brand\""), "BOM removed");
    assert_eq!(c.perfume_ref_id(&format!("{base}/Perfumes/Dior/Sauvage_Eau_de_Toilette?x=1")).unwrap(), "Dior/Sauvage_Eau_de_Toilette");
    assert_eq!(c.perfume_numeric_id("63363").await.unwrap(), (63363, None));
}

#[tokio::test]
async fn clients_of_one_user_share_their_request_slot() {
    let (base, _mock) = start().await;
    let cfg = ClientConfig { min_interval: Duration::from_millis(120), ..config(&base) };
    let limiter = UserLimiter::new();
    let public = ParfumoClient::with_limiter(cfg.clone(), GlobalLimiter::new(0), None, limiter.clone()).unwrap();
    let account = ParfumoClient::with_limiter(cfg, GlobalLimiter::new(0), None, limiter).unwrap();
    let started = Instant::now();
    public.csrf_key().await.unwrap();
    account.csrf_key().await.unwrap();
    public.shelf_default().await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(240), "three requests over two clients are spaced: {:?}", started.elapsed());
}
