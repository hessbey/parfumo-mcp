//! HTTP layer shared by `/mcp`, OAuth and the enrolment pages: state, bearer
//! authentication, local user login with throttling, HTML helpers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Response},
};

use parfumo_mcp_core::{
    crypto,
    db::{Db, User},
    state::{AppState, AuthUser},
};

use crate::{config::ServerConfig, i18n::Lang};

/// State of the HTTP server.
#[derive(Clone)]
pub struct ServerState {
    /// What the tools work with: their settings, the database, the Parfumo sessions.
    pub core: AppState,
    /// Settings of the HTTP side.
    pub http: Arc<ServerConfig>,
}

pub const LOGIN_WINDOW_SECS: i64 = 15 * 60;
/// Wrong access keys allowed from one client address in that window.
pub const LOGIN_MAX_FAILURES_PER_IP: i64 = 20;

/// At most `max` events per `per`, all callers together (in memory).
pub struct Window {
    max: usize,
    per: std::time::Duration,
    hits: std::sync::Mutex<std::collections::VecDeque<std::time::Instant>>,
}

impl Window {
    pub const fn new(max: usize, per: std::time::Duration) -> Self {
        Self { max, per, hits: std::sync::Mutex::new(std::collections::VecDeque::new()) }
    }

    /// Counts one event, unless the window is full.
    pub fn allow(&self) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|p| p.into_inner());
        let now = std::time::Instant::now();
        while hits.front().is_some_and(|t| now.duration_since(*t) >= self.per) {
            hits.pop_front();
        }
        if hits.len() >= self.max {
            return false;
        }
        hits.push_back(now);
        true
    }
}

/// Address of the client as the reverse proxy reports it: the last entry of
/// `X-Forwarded-For`, the one the proxy itself appended (Caddy discards what the client
/// sent). `None` without a proxy: the per-name lock still applies.
pub fn client_ip(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("x-forwarded-for")?.to_str().ok()?;
    value.rsplit(',').next()?.trim().parse::<std::net::IpAddr>().ok().map(|ip| ip.to_string())
}

pub async fn healthz(State(st): State<ServerState>) -> Response {
    if st.core.db.ping() {
        // The canary verdict is shown but never fails the health check: a container must
        // not be restarted because Parfumo changed a page.
        let canary = st.core.db.canary_last().ok().flatten().map(|run| serde_json::json!({"status": run.status, "at": run.ts}));
        Json(serde_json::json!({"status": "ok", "version": env!("CARGO_PKG_VERSION"), "canary": canary})).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"status": "database_unavailable"}))).into_response()
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token.trim()).filter(|t| !t.is_empty())
}

/// No anonymous request reaches `/mcp`. A 401 carries the protected-resource metadata URL
/// (RFC 9728) so that OAuth-capable clients can start the authorization flow.
pub async fn require_bearer(State(st): State<ServerState>, mut req: Request, next: Next) -> Response {
    let user = match bearer_token(req.headers()) {
        Some(token) => st.core.db.user_by_bearer_hash(&crypto::token_hash(token)).ok().flatten(),
        None => None,
    };
    match user {
        Some(user) => {
            req.extensions_mut().insert(AuthUser(user));
            next.run(req).await
        }
        None => {
            let had_token = bearer_token(req.headers()).is_some();
            let mut challenge = format!(
                "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\", scope=\"{}\"",
                st.http.public_url,
                crate::oauth::SCOPE
            );
            if had_token {
                challenge.push_str(", error=\"invalid_token\"");
            }
            let mut resp = (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "unauthorized"}))).into_response();
            if let Ok(v) = HeaderValue::from_str(&challenge) {
                resp.headers_mut().insert(header::WWW_AUTHENTICATE, v);
            }
            resp
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoginError {
    Invalid,
    Locked,
}

/// The user an access key belongs to. The key is the user's static bearer token: 256 random
/// bits, which cannot be guessed, so the only limit is on the load a client address may
/// cause: twenty wrong keys in fifteen minutes lock that address for the rest of the window.
pub fn verify_key(db: &Db, key: &str, ip: Option<&str>) -> Result<User, LoginError> {
    let ip_subject = ip.map(|ip| format!("ip:{ip}"));
    if ip_subject
        .as_deref()
        .is_some_and(|s| db.recent_login_failures(s, LOGIN_WINDOW_SECS).unwrap_or(LOGIN_MAX_FAILURES_PER_IP) >= LOGIN_MAX_FAILURES_PER_IP)
    {
        return Err(LoginError::Locked);
    }
    match db.user_by_key_hash(&crypto::token_hash(key.trim())).ok().flatten() {
        Some(user) => Ok(user),
        None => {
            if let Some(s) = &ip_subject {
                let _ = db.record_login_failure(s);
            }
            Err(LoginError::Invalid)
        }
    }
}

pub fn wrong_key(lang: Lang) -> &'static str {
    lang.pick("This access key is not valid.", "Cette clé d'accès n'est pas valide.", "Dieser Zugangsschlüssel ist ungültig.")
}

pub fn too_many_failures(lang: Lang) -> &'static str {
    lang.pick(
        "Too many failures: try again in fifteen minutes.",
        "Trop d'échecs : réessayez dans quinze minutes.",
        "Zu viele Fehlversuche: Versuche es in fünfzehn Minuten erneut.",
    )
}

/// CSRF defence for the HTML forms: a present `Origin` header must be this server.
///
/// A browser sends `Origin: null` with a form when the page forbids the referrer
/// (`Referrer-Policy: no-referrer`): [`page`] must therefore keep it for same-origin requests.
pub fn origin_ok(cfg: &ServerConfig, headers: &HeaderMap) -> bool {
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(o) => o.trim_end_matches('/').eq_ignore_ascii_case(&cfg.public_url),
    }
}

/// What the forms answer when `origin_ok` refuses a request: the address to use instead.
pub fn origin_refused(cfg: &ServerConfig, lang: Lang, path: &str) -> String {
    let url = format!("{}{path}", cfg.public_url);
    match lang {
        Lang::En => format!("Request refused: open this page at {url} and submit the form from there."),
        Lang::Fr => format!("Requête refusée : ouvrez cette page à l'adresse {url} et envoyez le formulaire depuis celle-ci."),
        Lang::De => format!("Anfrage abgelehnt: Öffne diese Seite unter {url} und sende das Formular von dort ab."),
    }
}

/// The language links of a page: `href` gives the address of the page in each language.
pub fn lang_switcher(current: Lang, href: impl Fn(Lang) -> String) -> String {
    let links: String = Lang::ALL
        .into_iter()
        .map(|lang| {
            let code = lang.code();
            if lang == current {
                format!("<a href=\"{}\" hreflang=\"{code}\" aria-current=\"true\">{}</a>", esc(&href(lang)), code.to_uppercase())
            } else {
                format!("<a href=\"{}\" hreflang=\"{code}\">{}</a>", esc(&href(lang)), code.to_uppercase())
            }
        })
        .collect();
    format!("<nav class=\"lang\" aria-label=\"Language\">{links}</nav>")
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// Style of the two pages a person sees (`/link` and the OAuth login): one narrow card,
/// in colours close to those of parfumo.com. No script and no external resource: the
/// content security policy of [`page`] allows neither.
const PAGE_CSS: &str = r#"
:root{--bg:#f0f1f6;--card:#fff;--ink:#22252e;--muted:#6b7080;--line:#dcdfe8;--accent:#3d6bd6;--accent-ink:#fff;
--ok-bg:#e7f4ec;--ok-ink:#1d5a36;--err-bg:#fcebe9;--err-ink:#9a2318}
@media (prefers-color-scheme:dark){:root{--bg:#14161b;--card:#1d2027;--ink:#eceef4;--muted:#9aa0af;--line:#353945;
--accent:#86a9f4;--accent-ink:#10131a;--ok-bg:#18301f;--ok-ink:#a6dcb8;--err-bg:#3a1f1c;--err-ink:#f3aea5}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:16px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
main{max-width:25rem;margin:0 auto;padding:1.25rem 1rem 2.5rem}
.lang{display:flex;justify-content:flex-end;gap:.25rem;margin:0 0 1rem}
.lang a{padding:.2rem .5rem;border-radius:.4rem;font-size:.78rem;font-weight:600;color:var(--muted);text-decoration:none}
.lang a[aria-current]{background:var(--card);color:var(--ink);box-shadow:0 0 0 1px var(--line)}
.card{background:var(--card);border-radius:.9rem;padding:1.5rem 1.4rem;box-shadow:0 1px 2px rgba(20,25,40,.06),0 0 0 1px var(--line)}
h1{margin:0 0 1.1rem;font-size:1.3rem;line-height:1.25}
h2{margin:1.1rem 0 .2rem;font-size:.9rem}
p{margin:0 0 .9rem}
label{display:block;margin:.9rem 0 .3rem;font-size:.86rem;font-weight:600}
input[type=text],input[type=password],textarea{width:100%;padding:.6rem .7rem;border:1px solid var(--line);border-radius:.55rem;
background:var(--card);color:var(--ink);font:inherit}
textarea{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.85rem;resize:vertical}
input:focus-visible,textarea:focus-visible,button:focus-visible,summary:focus-visible,a:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.hint{margin:.3rem 0 0;font-size:.8rem;color:var(--muted)}
.check{display:flex;gap:.6rem;align-items:flex-start;margin:1.1rem 0 0;font-weight:400;font-size:.86rem;cursor:pointer}
.check input{margin:.2rem 0 0;accent-color:var(--accent)}
button{display:block;width:100%;margin:1.1rem 0 0;padding:.7rem 1rem;border:0;border-radius:.55rem;background:var(--accent);
color:var(--accent-ink);font:inherit;font-weight:600;cursor:pointer}
button.quiet{margin-top:.7rem;background:transparent;color:var(--ink);box-shadow:inset 0 0 0 1px var(--line);font-weight:500;font-size:.9rem}
button:hover{filter:brightness(1.07)}
.pair{display:grid;grid-template-columns:1fr 1fr;gap:.6rem}
details{margin:1.3rem 0 0;padding-top:1rem;border-top:1px solid var(--line)}
summary{font-size:.86rem;color:var(--muted);cursor:pointer}
.msg{margin:0 0 1rem;padding:.7rem .9rem;border-radius:.55rem;background:var(--ok-bg);color:var(--ok-ink);font-size:.92rem}
.msg.err{background:var(--err-bg);color:var(--err-ink)}
.lead{color:var(--muted);font-size:.92rem}
.lead strong{color:var(--ink)}
.to{margin:0 0 .9rem;padding:.6rem .7rem;border-radius:.55rem;background:var(--bg);font-size:.82rem;overflow-wrap:anywhere}
.to span{display:block;color:var(--muted)}
code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.92em}
.foot{margin:1.1rem .4rem 0;font-size:.76rem;color:var(--muted);text-align:center}
"#;

/// A page of this server: the language links, then one card holding the title and `body`.
/// A paragraph of class `foot` in `body` is shown under the card.
pub fn page(status: StatusCode, lang: Lang, title: &str, switcher: &str, body: &str) -> Response {
    let (card, foot) = match body.rfind("<p class=\"foot\">") {
        Some(at) => body.split_at(at),
        None => (body, ""),
    };
    let html = format!(
        "<!doctype html><html lang=\"{code}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex\"><meta name=\"color-scheme\" content=\"light dark\"><title>{t} · Parfumo MCP</title>\
         <style>{PAGE_CSS}</style></head><body><main>{switcher}<div class=\"card\"><h1>{t}</h1>{card}</div>{foot}</main></body></html>",
        code = lang.code(),
        t = esc(title)
    );
    let mut resp = (status, Html(html)).into_response();
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    // Nothing leaks to other sites, and the forms of these pages keep their `Origin` header,
    // which `origin_ok` checks: with `no-referrer` a browser would send `Origin: null`.
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    h.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'"),
    );
    resp
}
