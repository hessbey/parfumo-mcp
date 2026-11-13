//! The login page of the local server: where the one user of this computer links a
//! Parfumo account, instead of writing it in the configuration of the MCP client.
//!
//! The page is opened on demand and stays open for ten minutes, or until an account is
//! linked. It listens on 127.0.0.1 only, on a port chosen by the system, under an address
//! that holds a secret: nothing else on the network, and no other site in the browser, can
//! reach the form. The form itself and what its buttons do are those of the `/link` page
//! of the HTTP server.

use std::{
    net::Ipv4Addr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use parfumo_mcp_core::{
    crypto,
    pages::{
        self, Lang,
        link::{self, LinkForm, Place},
    },
    sessions::Sessions,
};
use serde::Deserialize;
use tokio::sync::watch;

/// How long the page stays open when nobody links an account.
pub const OPEN_FOR: Duration = Duration::from_secs(600);

/// What became of an opened page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Still open.
    Open,
    /// An account was linked: the page closed.
    Linked,
    /// Nobody linked an account in time: the page closed.
    Expired,
}

/// The login page of one local user. Cheap to clone.
#[derive(Clone)]
pub struct LoginPage {
    sessions: Arc<Sessions>,
    user_id: i64,
    current: Arc<Mutex<Option<Arc<Opened>>>>,
}

/// One opening of the page.
struct Opened {
    sessions: Arc<Sessions>,
    user_id: i64,
    port: u16,
    /// The secret part of the address.
    token: String,
    until: Mutex<Instant>,
    ending: watch::Sender<Ending>,
}

impl Opened {
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/{}", self.port, self.token)
    }

    /// Closes the page unless an account was linked.
    fn expire(&self) {
        self.ending.send_if_modified(|ending| {
            let open = *ending == Ending::Open;
            if open {
                *ending = Ending::Expired;
            }
            open
        });
    }

    /// Resolves when an account is linked, or when the time is up.
    async fn ended(&self) {
        let mut ending = self.ending.subscribe();
        loop {
            let until = *self.until.lock().unwrap_or_else(|p| p.into_inner());
            tokio::select! {
                _ = ending.wait_for(|e| *e != Ending::Open) => return,
                _ = tokio::time::sleep_until(until.into()) => {
                    // The time may have been extended while this slept.
                    if Instant::now() >= *self.until.lock().unwrap_or_else(|p| p.into_inner()) {
                        self.expire();
                        return;
                    }
                }
            }
        }
    }
}

impl LoginPage {
    pub fn new(sessions: Arc<Sessions>, user_id: i64) -> Self {
        Self { sessions, user_id, current: Arc::default() }
    }

    /// The address of the page, which is opened if it is not, and stays open ten more
    /// minutes. `None` when it cannot be opened. Must be called inside the Tokio runtime.
    pub fn url(&self) -> Option<String> {
        let mut current = self.current.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(opened) = current.as_ref().filter(|o| *o.ending.borrow() == Ending::Open) {
            *opened.until.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now() + OPEN_FOR;
            return Some(opened.url());
        }
        match self.open() {
            Ok(opened) => {
                let url = opened.url();
                *current = Some(opened);
                Some(url)
            }
            Err(e) => {
                tracing::warn!(error = %e, "the local login page could not be opened");
                None
            }
        }
    }

    /// Waits until the page that [`LoginPage::url`] opened is closed, and tells why.
    pub async fn ended(&self) -> Ending {
        let opened = self.current.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let Some(opened) = opened else { return Ending::Expired };
        opened.ended().await;
        *opened.ending.borrow()
    }

    fn open(&self) -> std::io::Result<Arc<Opened>> {
        // This computer only, on a port the system picks.
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let listener = tokio::net::TcpListener::from_std(listener)?;
        let opened = Arc::new(Opened {
            sessions: self.sessions.clone(),
            user_id: self.user_id,
            port,
            token: crypto::new_secret(""),
            until: Mutex::new(Instant::now() + OPEN_FOR),
            ending: watch::Sender::new(Ending::Open),
        });
        let app = Router::new().route("/{token}", get(show).post(submit)).with_state(opened.clone());
        let page = opened.clone();
        tokio::spawn(async move {
            let ended = page.clone();
            if let Err(e) = axum::serve(listener, app).with_graceful_shutdown(async move { ended.ended().await }).await {
                tracing::warn!(error = %e, "the local login page stopped on an error");
            }
            page.expire();
            tracing::info!(port = page.port, "local login page closed");
        });
        tracing::info!(port, "local login page open on 127.0.0.1");
        Ok(opened)
    }
}

/// Opens `url` in the default browser. Returns whether a browser could be started.
pub fn open_browser(url: &str) -> bool {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    } else {
        Command::new("xdg-open")
    };
    command.arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

// ---- the page itself -----------------------------------------------------------------

fn respond(status: StatusCode, lang: Lang, switcher: &str, body: &str) -> Response {
    let mut response = (status, Html(pages::html(lang, link::title(lang), switcher, body))).into_response();
    for (name, value) in pages::HEADERS {
        response.headers_mut().insert(name, HeaderValue::from_static(value));
    }
    response
}

/// The form, with `message` above it.
fn form(page: &Opened, status: StatusCode, lang: Lang, message: &str, more_open: bool) -> Response {
    let path = format!("/{}", page.token);
    let switcher = pages::lang_switcher(lang, |l| format!("{path}?lang={}", l.code()));
    respond(status, lang, &switcher, &link::form(lang, Place::Computer, &path, message, more_open))
}

fn header(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// The names this page answers to: its own address, as a browser on this computer writes
/// it. Any other `Host` is a name that was made to point here (DNS rebinding).
fn own_names(port: u16) -> [String; 2] {
    [format!("127.0.0.1:{port}"), format!("localhost:{port}")]
}

/// Whether a request is for this page: its own host name, and the secret of its address.
/// Anything else gets the same empty 404.
fn is_for(page: &Opened, token: &str, headers: &HeaderMap) -> bool {
    let host = header(headers, header::HOST).unwrap_or_default();
    own_names(page.port).iter().any(|name| name.eq_ignore_ascii_case(host)) && crypto::ct_eq(token, &page.token)
}

/// Whether a form comes from this page: a browser names the page it was sent from.
fn same_origin(page: &Opened, headers: &HeaderMap) -> bool {
    match header(headers, header::ORIGIN) {
        None => true,
        Some(origin) => own_names(page.port).iter().any(|name| format!("http://{name}").eq_ignore_ascii_case(origin.trim_end_matches('/'))),
    }
}

#[derive(Deserialize)]
struct LangQuery {
    #[serde(default)]
    lang: Option<String>,
}

async fn show(State(page): State<Arc<Opened>>, Path(token): Path<String>, Query(q): Query<LangQuery>, headers: HeaderMap) -> Response {
    if !is_for(&page, &token, &headers) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let lang = Lang::choose(q.lang.as_deref(), header(&headers, header::ACCEPT_LANGUAGE));
    form(&page, StatusCode::OK, lang, "", false)
}

async fn submit(State(page): State<Arc<Opened>>, Path(token): Path<String>, headers: HeaderMap, Form(f): Form<LinkForm>) -> Response {
    if !is_for(&page, &token, &headers) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let lang = Lang::choose(f.lang.as_deref(), header(&headers, header::ACCEPT_LANGUAGE));
    if !same_origin(&page, &headers) {
        return form(&page, StatusCode::FORBIDDEN, lang, &pages::msg(false, &pages::origin_refused(lang, &page.url())), f.more_open());
    }
    let outcome = link::perform(&page.sessions, page.user_id, lang, Place::Computer, &f).await;
    tracing::info!(action = %f.action, ok = outcome.ok, "local login page action");
    if outcome.linked {
        // Nothing more to type: the page closes once this answer is sent.
        page.ending.send_replace(Ending::Linked);
        return respond(StatusCode::OK, lang, "", &link::done(lang, &outcome.message()));
    }
    form(&page, StatusCode::from_u16(outcome.status).unwrap_or(StatusCode::OK), lang, &outcome.message(), f.more_open())
}
