//! Client-to-server authorization: a minimal OAuth 2.1 authorization server for public
//! clients (authorization code + PKCE S256, rotating refresh tokens), following the MCP
//! authorization specification:
//!
//! - RFC 9728 protected resource metadata, RFC 8414 authorization server metadata;
//! - Client ID Metadata Documents first, RFC 7591 dynamic registration kept for compatibility;
//! - no open sign-up: the resource owner logs in with a user created by `parfumo-mcp-server user add`.
//!
//! Tokens are opaque random strings stored as SHA-256 hashes. This is unrelated to the
//! Parfumo account: nothing here ever sees a Parfumo credential.

use std::{net::IpAddr, time::Duration};

use axum::{
    Form, Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use url::Url;

use parfumo_mcp_core::{
    crypto,
    db::{AuthCode, OAuthClient, RefreshTake},
};

use crate::{
    i18n::Lang,
    web::{
        LoginError, ServerState, Window, client_ip, esc, lang_switcher, origin_ok, origin_refused, page, too_many_failures, verify_key, wrong_key,
    },
};

pub const SCOPE: &str = "parfumo";
const CODE_TTL_SECS: i64 = 60;
const CIMD_MAX_BYTES: usize = 64 * 1024;
const MAX_REDIRECT_URIS: usize = 10;

// ---- metadata ------------------------------------------------------------------------------

pub async fn protected_resource_metadata(State(st): State<ServerState>) -> Json<serde_json::Value> {
    Json(json!({
        "resource": st.http.resource_url(),
        "authorization_servers": [st.http.public_url],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "Parfumo MCP server",
    }))
}

pub async fn authorization_server_metadata(State(st): State<ServerState>) -> Json<serde_json::Value> {
    let base = &st.http.public_url;
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": [SCOPE],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
    }))
}

// ---- clients -------------------------------------------------------------------------------

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

/// Accepted redirect URIs: `https://…`, `http://` on loopback only, or a private-use
/// scheme of a native application. Never a fragment, never a script-capable scheme.
pub fn redirect_uri_allowed(uri: &str) -> bool {
    let Ok(u) = Url::parse(uri) else { return false };
    if u.fragment().is_some() {
        return false;
    }
    match u.scheme() {
        "https" => u.host_str().is_some(),
        "http" => u.host_str().is_some_and(is_loopback_host),
        "javascript" | "data" | "file" | "vbscript" | "blob" | "about" => false,
        _ => true,
    }
}

/// Exact match, except that a loopback redirect may use any port (native applications
/// listen on an ephemeral port).
pub fn redirect_matches(registered: &str, requested: &str) -> bool {
    if registered == requested {
        return true;
    }
    let (Ok(a), Ok(b)) = (Url::parse(registered), Url::parse(requested)) else { return false };
    a.scheme() == "http"
        && b.scheme() == "http"
        && a.host_str().is_some_and(is_loopback_host)
        && a.host_str() == b.host_str()
        && a.path() == b.path()
        && a.query() == b.query()
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                // Protocol assignments and benchmarking networks.
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (18..20).contains(&o[1]))
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let s = v6.segments();
            // NAT64 and 6to4 embed an IPv4 address that the checks above would not see.
            let embeds_v4 = (s[0] == 0x64 && s[1] == 0xff9b) || s[0] == 0x2002;
            let documentation = s[0] == 0x2001 && s[1] == 0x0db8;
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || embeds_v4
                || documentation)
        }
    }
}

/// A Client ID Metadata Document URL: `https`, a DNS name, a path, no credentials or fragment.
pub fn validate_cimd_url(client_id: &str) -> Result<Url, String> {
    let u = Url::parse(client_id).map_err(|_| "client_id is not a valid URL".to_string())?;
    if u.scheme() != "https" {
        return Err("client_id metadata documents must use https".into());
    }
    if !u.username().is_empty() || u.password().is_some() || u.fragment().is_some() {
        return Err("client_id URL must not contain credentials or a fragment".into());
    }
    if u.path().is_empty() || u.path() == "/" {
        return Err("client_id URL must contain a path".into());
    }
    match u.host() {
        Some(url::Host::Domain(d)) if !is_loopback_host(d) && d.contains('.') => Ok(u),
        _ => Err("client_id URL must use a public DNS name".into()),
    }
}

/// Validates the fetched document: its `client_id` must be the URL it was fetched from.
pub fn parse_cimd(client_id: &str, body: &[u8]) -> Result<OAuthClient, String> {
    #[derive(Deserialize)]
    struct Doc {
        client_id: String,
        #[serde(default)]
        client_name: Option<String>,
        redirect_uris: Vec<String>,
    }
    let doc: Doc =
        serde_json::from_slice(body).map_err(|_| "client metadata document is not valid JSON with client_id and redirect_uris".to_string())?;
    if doc.client_id != client_id {
        return Err("client metadata document client_id does not match its URL".into());
    }
    validate_redirect_uris(&doc.redirect_uris)?;
    Ok(OAuthClient {
        client_id: doc.client_id,
        client_name: doc.client_name.map(|n| n.chars().take(100).collect()),
        redirect_uris: doc.redirect_uris,
    })
}

fn validate_redirect_uris(uris: &[String]) -> Result<(), String> {
    if uris.is_empty() || uris.len() > MAX_REDIRECT_URIS {
        return Err(format!("redirect_uris must contain 1 to {MAX_REDIRECT_URIS} entries"));
    }
    match uris.iter().find(|u| !redirect_uri_allowed(u)) {
        Some(bad) => Err(format!("redirect_uri not allowed: {bad}")),
        None => Ok(()),
    }
}

/// Fetches a Client ID Metadata Document with SSRF defences: public addresses only, the
/// connection pinned to the address that was checked, no redirect, short timeout, size cap.
async fn fetch_cimd(client_id: &str) -> Result<OAuthClient, String> {
    let url = validate_cimd_url(client_id)?;
    let host = url.host_str().unwrap_or_default().to_string();
    let port = url.port().unwrap_or(443);
    let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|_| "client_id host does not resolve".to_string())?.collect();
    let addr = *addrs.first().ok_or_else(|| "client_id host does not resolve".to_string())?;
    if !addrs.iter().all(|a| is_public_ip(a.ip())) {
        return Err("client_id host resolves to a non-public address".into());
    }
    let http = reqwest::Client::builder()
        .use_preconfigured_tls(parfumo_client::tls_config())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .resolve(&host, addr)
        .user_agent(concat!("parfumo-mcp/", env!("CARGO_PKG_VERSION"), " (client-id-metadata-document fetch)"))
        .build()
        .map_err(|_| "cannot build HTTP client".to_string())?;
    let mut resp =
        http.get(url).header("accept", "application/json").send().await.map_err(|_| "client metadata document could not be fetched".to_string())?;
    if resp.status() != reqwest::StatusCode::OK {
        return Err(format!("client metadata document returned HTTP {}", resp.status().as_u16()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| "client metadata document could not be read".to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() > CIMD_MAX_BYTES {
            return Err("client metadata document is too large".into());
        }
    }
    parse_cimd(client_id, &body)
}

const CIMD_MAX_URL_CHARS: usize = 512;
const CIMD_FAILURE_TTL: Duration = Duration::from_secs(600);
/// Client metadata documents fetched per minute, all visitors together.
static CIMD_FETCHES: Window = Window::new(10, Duration::from_secs(60));
/// Dynamic client registrations per minute, all visitors together.
static REGISTRATIONS: Window = Window::new(30, Duration::from_secs(60));

async fn resolve_client(st: &ServerState, client_id: &str) -> Result<OAuthClient, String> {
    if client_id.starts_with("https://") {
        let key = format!("cimd:{client_id}");
        if let Some(cached) = st.core.db.cache_get(&key).and_then(|c| serde_json::from_str::<Vec<String>>(&c).ok()) {
            return Ok(OAuthClient { client_id: client_id.to_string(), client_name: None, redirect_uris: cached });
        }
        // An anonymous visitor chooses this URL: a document that failed is not fetched
        // again for a while, and the server as a whole fetches only a few per minute.
        if client_id.len() > CIMD_MAX_URL_CHARS {
            return Err("client_id URL is too long".into());
        }
        let failed = format!("cimd-failed:{client_id}");
        if let Some(why) = st.core.db.cache_get(&failed) {
            return Err(why);
        }
        if !CIMD_FETCHES.allow() {
            return Err("too many client metadata documents are being fetched; retry in a minute".into());
        }
        let client = match fetch_cimd(client_id).await {
            Ok(c) => c,
            Err(why) => {
                st.core.db.cache_put(&failed, &why, CIMD_FAILURE_TTL);
                return Err(why);
            }
        };
        st.core.db.cache_put(&key, &serde_json::to_string(&client.redirect_uris).unwrap_or_default(), Duration::from_secs(3600));
        return Ok(client);
    }
    st.core.db.oauth_client_get(client_id).ok().flatten().ok_or_else(|| "unknown client_id".to_string())
}

/// RFC 7591 dynamic client registration (public clients only). Deprecated by the
/// 2026-07-28 specification in favour of Client ID Metadata Documents, kept for older clients.
pub async fn register(State(st): State<ServerState>, Json(body): Json<serde_json::Value>) -> Response {
    let bad = |desc: &str| (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_client_metadata", "error_description": desc}))).into_response();
    let Some(uris) = body.get("redirect_uris").and_then(|v| v.as_array()) else { return bad("redirect_uris is required") };
    let uris: Vec<String> = uris.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
    if let Err(e) = validate_redirect_uris(&uris) {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_redirect_uri", "error_description": e}))).into_response();
    }
    if let Some(m) = body.get("token_endpoint_auth_method").and_then(|v| v.as_str())
        && m != "none"
    {
        return bad("only public clients are supported (token_endpoint_auth_method must be none)");
    }
    if uris.iter().any(|u| u.len() > CIMD_MAX_URL_CHARS) {
        return bad("a redirect_uri is too long");
    }
    // Registration is open to anyone by design: bound what it can store.
    let busy = || {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            Json(json!({"error": "temporarily_unavailable", "error_description": "too many client registrations; retry later"})),
        )
            .into_response()
    };
    if !REGISTRATIONS.allow() {
        return busy();
    }
    if st.core.db.oauth_client_count().unwrap_or(parfumo_mcp_core::db::MAX_OAUTH_CLIENTS) >= parfumo_mcp_core::db::MAX_OAUTH_CLIENTS {
        let _ = st.core.db.purge();
        if st.core.db.oauth_client_count().unwrap_or(parfumo_mcp_core::db::MAX_OAUTH_CLIENTS) >= parfumo_mcp_core::db::MAX_OAUTH_CLIENTS {
            tracing::warn!("oauth client registration refused: the table is full");
            return busy();
        }
    }
    let client = OAuthClient {
        client_id: crypto::new_secret("pmcl_"),
        client_name: body.get("client_name").and_then(|v| v.as_str()).map(|n| n.chars().take(100).collect()),
        redirect_uris: uris,
    };
    if st.core.db.oauth_client_put(&client).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "server_error"}))).into_response();
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": client.client_id,
            "client_name": client.client_name,
            "redirect_uris": client.redirect_uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "client_id_issued_at": parfumo_mcp_core::db::now(),
        })),
    )
        .into_response()
}

// ---- authorization endpoint ----------------------------------------------------------------

#[derive(Deserialize, Default, Clone)]
pub struct AuthorizeParams {
    #[serde(default)]
    response_type: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    code_challenge: String,
    #[serde(default)]
    code_challenge_method: String,
    #[serde(default)]
    resource: Option<String>,
    /// The user's access key, typed on the login page.
    #[serde(default)]
    access_key: String,
    /// Language of the login page.
    #[serde(default)]
    lang: Option<String>,
}

enum AuthzError {
    /// Client or redirect URI not trusted: never redirect, show an error page.
    Page(String),
    /// Redirect URI verified: report the error to the client.
    Redirect(&'static str, &'static str),
}

fn redirect_with(st: &ServerState, redirect_uri: &str, state: Option<&str>, pairs: &[(&str, &str)]) -> Response {
    let Ok(mut u) = Url::parse(redirect_uri) else {
        return error_page(StatusCode::BAD_REQUEST, Lang::En, "Invalid redirect_uri.");
    };
    {
        let mut q = u.query_pairs_mut();
        for (k, v) in pairs {
            q.append_pair(k, v);
        }
        if let Some(s) = state {
            q.append_pair("state", s);
        }
        q.append_pair("iss", &st.http.public_url);
    }
    let mut resp = StatusCode::FOUND.into_response();
    match HeaderValue::from_str(u.as_str()) {
        Ok(v) => {
            resp.headers_mut().insert(header::LOCATION, v);
            resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            resp
        }
        Err(_) => error_page(StatusCode::BAD_REQUEST, Lang::En, "Invalid redirect_uri."),
    }
}

async fn validate(st: &ServerState, p: &AuthorizeParams) -> Result<OAuthClient, AuthzError> {
    if p.client_id.is_empty() || p.redirect_uri.is_empty() {
        return Err(AuthzError::Page("client_id et redirect_uri sont requis.".into()));
    }
    let client = resolve_client(st, &p.client_id).await.map_err(|e| AuthzError::Page(format!("OAuth client refused: {e}")))?;
    if !client.redirect_uris.iter().any(|r| redirect_matches(r, &p.redirect_uri)) || !redirect_uri_allowed(&p.redirect_uri) {
        return Err(AuthzError::Page("redirect_uri is not registered for this client.".into()));
    }
    if p.response_type != "code" {
        return Err(AuthzError::Redirect("unsupported_response_type", "only response_type=code is supported"));
    }
    if p.code_challenge_method != "S256" || p.code_challenge.len() < 43 || p.code_challenge.len() > 128 {
        return Err(AuthzError::Redirect("invalid_request", "PKCE with code_challenge_method=S256 is required"));
    }
    if let Some(r) = p.resource.as_deref()
        && r.trim_end_matches('/') != st.http.resource_url()
        && r.trim_end_matches('/') != st.http.public_url
    {
        return Err(AuthzError::Redirect("invalid_target", "unknown resource"));
    }
    Ok(client)
}

fn authz_title(lang: Lang) -> &'static str {
    lang.pick("Authorize access", "Autoriser l'accès", "Zugriff erlauben")
}

fn error_page(status: StatusCode, lang: Lang, text: &str) -> Response {
    page(status, lang, authz_title(lang), "", &format!("<p class=\"msg err\">{}</p>", esc(text)))
}

/// The OAuth parameters of a request, to carry them through the login form and the
/// language links.
fn oauth_pairs(p: &AuthorizeParams) -> Vec<(&'static str, &str)> {
    [
        ("response_type", p.response_type.as_str()),
        ("client_id", p.client_id.as_str()),
        ("redirect_uri", p.redirect_uri.as_str()),
        ("state", p.state.as_deref().unwrap_or("")),
        ("code_challenge", p.code_challenge.as_str()),
        ("code_challenge_method", p.code_challenge_method.as_str()),
        ("resource", p.resource.as_deref().unwrap_or("")),
    ]
    .into_iter()
    .filter(|(k, v)| !(v.is_empty() && matches!(*k, "state" | "resource")))
    .collect()
}

fn login_page(status: StatusCode, lang: Lang, p: &AuthorizeParams, client: &OAuthClient, error: Option<&str>) -> Response {
    let t = |en, fr, de| lang.pick(en, fr, de);
    let pairs = oauth_pairs(p);
    let hidden: String = pairs.iter().map(|(k, v)| format!("<input type=\"hidden\" name=\"{k}\" value=\"{}\">", esc(v))).collect();
    let switcher = lang_switcher(lang, |l| {
        let mut q = url::form_urlencoded::Serializer::new(String::new());
        q.extend_pairs(pairs.iter().copied()).append_pair("lang", l.code());
        format!("/oauth/authorize?{}", q.finish())
    });
    // A metadata-document client is identified by an https URL it controls. A dynamically
    // registered client only declares a name: anyone can register one called "Claude".
    let who = if client.client_id.starts_with("https://") {
        format!("<strong>{}</strong>", esc(&client.client_id))
    } else {
        format!(
            "<strong>{}</strong> {}",
            esc(client.client_name.as_deref().unwrap_or("?")),
            t(
                "(name declared by the application itself, not verified)",
                "(nom déclaré par l'application elle-même, non vérifié)",
                "(Name von der Anwendung selbst angegeben, nicht geprüft)"
            )
        )
    };
    let lead = match lang {
        Lang::En => format!("The application {who} asks for access to this Parfumo MCP server on your behalf."),
        Lang::Fr => format!("L'application {who} demande l'accès à ce serveur MCP Parfumo en votre nom."),
        Lang::De => format!("Die Anwendung {who} bittet in deinem Namen um Zugriff auf diesen Parfumo-MCP-Server."),
    };
    let body = format!(
        r#"{err}<p class="lead">{lead}</p>
<p class="to"><span>{back}</span><code>{redirect}</code></p>
<p class="hint">{warn}</p>
<form method="post" action="/oauth/authorize" autocomplete="off">{hidden}
<input type="hidden" name="lang" value="{code}">
<label for="k">{key}</label>
<input id="k" type="password" name="access_key" required autofocus placeholder="pmcp_…">
<p class="hint">{key_hint}</p>
<button type="submit">{authorize}</button>
</form>"#,
        err = error.map(|e| format!("<p class=\"msg err\">{}</p>", esc(e))).unwrap_or_default(),
        back = t(
            "Once you authorize, you will be sent back to",
            "Après autorisation, vous serez renvoyé vers",
            "Nach der Freigabe wirst du weitergeleitet zu"
        ),
        redirect = esc(&p.redirect_uri),
        warn = t(
            "Authorize only if you have just started this connection yourself and this address is really that of your application.",
            "N'autorisez que si vous venez de lancer cette connexion vous-même et si cette adresse est bien celle de votre application.",
            "Erlaube den Zugriff nur, wenn du diese Verbindung gerade selbst gestartet hast und diese Adresse wirklich zu deiner Anwendung gehört."
        ),
        code = lang.code(),
        key = t("Access key", "Clé d'accès", "Zugangsschlüssel"),
        key_hint = t(
            "Given to you by the person who runs this server. It is not your Parfumo password.",
            "Fournie par la personne qui gère ce serveur. Ce n'est pas votre mot de passe Parfumo.",
            "Du hast ihn von der Person erhalten, die diesen Server betreibt. Es ist nicht dein Parfumo-Passwort."
        ),
        authorize = t("Authorize", "Autoriser", "Erlauben"),
    );
    page(status, lang, authz_title(lang), &switcher, &body)
}

fn authz_error(st: &ServerState, lang: Lang, p: &AuthorizeParams, e: AuthzError) -> Response {
    match e {
        AuthzError::Page(m) => error_page(StatusCode::BAD_REQUEST, lang, &m),
        AuthzError::Redirect(code, desc) => redirect_with(st, &p.redirect_uri, p.state.as_deref(), &[("error", code), ("error_description", desc)]),
    }
}

pub async fn authorize_get(State(st): State<ServerState>, headers: HeaderMap, Query(p): Query<AuthorizeParams>) -> Response {
    let lang = Lang::choose(p.lang.as_deref(), &headers);
    match validate(&st, &p).await {
        Ok(client) => login_page(StatusCode::OK, lang, &p, &client, None),
        Err(e) => authz_error(&st, lang, &p, e),
    }
}

pub async fn authorize_post(State(st): State<ServerState>, headers: HeaderMap, Form(p): Form<AuthorizeParams>) -> Response {
    let lang = Lang::choose(p.lang.as_deref(), &headers);
    if !origin_ok(&st.http, &headers) {
        return error_page(StatusCode::FORBIDDEN, lang, &origin_refused(&st.http, lang, "/oauth/authorize"));
    }
    let client = match validate(&st, &p).await {
        Ok(c) => c,
        Err(e) => return authz_error(&st, lang, &p, e),
    };
    let user = match verify_key(&st.core.db, &p.access_key, client_ip(&headers).as_deref()) {
        Ok(u) => u,
        Err(LoginError::Locked) => return login_page(StatusCode::TOO_MANY_REQUESTS, lang, &p, &client, Some(too_many_failures(lang))),
        Err(LoginError::Invalid) => return login_page(StatusCode::UNAUTHORIZED, lang, &p, &client, Some(wrong_key(lang))),
    };
    let code = crypto::new_secret("pmac_");
    let record =
        AuthCode { client_id: p.client_id.clone(), user_id: user.id, redirect_uri: p.redirect_uri.clone(), code_challenge: p.code_challenge.clone() };
    if st.core.db.oauth_code_put(&crypto::token_hash(&code), &record, CODE_TTL_SECS).is_err() {
        return redirect_with(&st, &p.redirect_uri, p.state.as_deref(), &[("error", "server_error")]);
    }
    tracing::info!(user = %user.name, client_id = %p.client_id, "oauth authorization granted");
    redirect_with(&st, &p.redirect_uri, p.state.as_deref(), &[("code", &code)])
}

// ---- token endpoint ------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct TokenForm {
    #[serde(default)]
    grant_type: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    code_verifier: String,
    #[serde(default)]
    refresh_token: String,
}

fn token_error(error: &str, desc: &str) -> Response {
    let mut r = (StatusCode::BAD_REQUEST, Json(json!({"error": error, "error_description": desc}))).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn issue_tokens(st: &ServerState, client_id: &str, user_id: i64, family: &str) -> Response {
    let access = crypto::new_secret("pmat_");
    let refresh = crypto::new_secret("pmrt_");
    let stored =
        st.core.db.oauth_token_put(&crypto::token_hash(&access), "access", client_id, user_id, st.http.access_token_ttl, family).and_then(|()| {
            st.core.db.oauth_token_put(&crypto::token_hash(&refresh), "refresh", client_id, user_id, st.http.refresh_token_ttl, family)
        });
    if stored.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "server_error"}))).into_response();
    }
    let mut r = Json(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": st.http.access_token_ttl.as_secs(),
        "refresh_token": refresh,
        "scope": SCOPE,
    }))
    .into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

pub async fn token(State(st): State<ServerState>, Form(f): Form<TokenForm>) -> Response {
    match f.grant_type.as_str() {
        "authorization_code" => {
            if f.code.is_empty() || f.code_verifier.is_empty() || f.client_id.is_empty() {
                return token_error("invalid_request", "code, code_verifier and client_id are required");
            }
            let Ok(Some(code)) = st.core.db.oauth_code_take(&crypto::token_hash(&f.code)) else {
                return token_error("invalid_grant", "authorization code is invalid, expired or already used");
            };
            if code.client_id != f.client_id || code.redirect_uri != f.redirect_uri {
                return token_error("invalid_grant", "client_id or redirect_uri does not match the authorization request");
            }
            if !(43..=128).contains(&f.code_verifier.len()) || !crypto::ct_eq(&crypto::pkce_s256(&f.code_verifier), &code.code_challenge) {
                return token_error("invalid_grant", "PKCE verification failed");
            }
            issue_tokens(&st, &code.client_id, code.user_id, &crypto::new_secret("fam_"))
        }
        "refresh_token" => {
            let (client_id, user_id, family) = match st.core.db.oauth_refresh_take(&crypto::token_hash(&f.refresh_token)) {
                Ok(RefreshTake::Valid { client_id, user_id, family }) => (client_id, user_id, family),
                Ok(RefreshTake::Reused) => {
                    tracing::warn!("a refresh token was presented twice: every token of that authorization is revoked");
                    return token_error("invalid_grant", "refresh token was already used; the authorization is revoked, log in again");
                }
                _ => return token_error("invalid_grant", "refresh token is invalid, expired or already used"),
            };
            if !f.client_id.is_empty() && f.client_id != client_id {
                return token_error("invalid_grant", "refresh token was issued to another client");
            }
            issue_tokens(&st, &client_id, user_id, &family)
        }
        _ => token_error("unsupported_grant_type", "supported: authorization_code, refresh_token"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uri_rules() {
        assert!(redirect_uri_allowed("https://claude.ai/api/mcp/auth_callback"));
        assert!(redirect_uri_allowed("http://localhost:6274/oauth/callback"));
        assert!(redirect_uri_allowed("http://127.0.0.1/cb"));
        assert!(redirect_uri_allowed("cursor://anysphere.cursor-mcp/oauth/callback"));
        assert!(!redirect_uri_allowed("http://evil.example/cb"));
        assert!(!redirect_uri_allowed("javascript:alert(1)"));
        assert!(!redirect_uri_allowed("https://a.example/cb#frag"));
        assert!(!redirect_uri_allowed("not a url"));

        assert!(redirect_matches("https://a.example/cb", "https://a.example/cb"));
        assert!(!redirect_matches("https://a.example/cb", "https://a.example/cb2"));
        assert!(!redirect_matches("https://a.example/cb", "https://a.example:444/cb"));
        assert!(redirect_matches("http://localhost/cb", "http://localhost:51234/cb"));
        assert!(!redirect_matches("http://localhost/cb", "http://localhost:51234/other"));
        assert!(!redirect_matches("http://localhost/cb", "http://127.0.0.1:51234/cb"));
    }

    #[test]
    fn cimd_url_and_document_rules() {
        assert!(validate_cimd_url("https://app.example.com/oauth/client.json").is_ok());
        for bad in [
            "http://app.example.com/client.json",
            "https://app.example.com/",
            "https://localhost/client.json",
            "https://127.0.0.1/client.json",
            "https://[::1]/client.json",
            "https://user:pw@app.example.com/c.json",
            "https://app.example.com/c.json#x",
            "https://intranet/c.json",
        ] {
            assert!(validate_cimd_url(bad).is_err(), "{bad} must be rejected");
        }
        let id = "https://app.example.com/client.json";
        let ok = format!(r#"{{"client_id":"{id}","client_name":"App","redirect_uris":["https://app.example.com/cb"]}}"#);
        assert_eq!(parse_cimd(id, ok.as_bytes()).unwrap().redirect_uris, vec!["https://app.example.com/cb"]);
        let other = ok.replace("client.json\"", "other.json\"");
        assert!(parse_cimd(id, other.as_bytes()).is_err(), "client_id must equal the document URL");
        let evil = format!(r#"{{"client_id":"{id}","redirect_uris":["http://evil.example/cb"]}}"#);
        assert!(parse_cimd(id, evil.as_bytes()).is_err());
        assert!(parse_cimd(id, b"<html>").is_err());
    }

    #[test]
    fn private_addresses_are_not_fetched() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
            "192.0.0.8",
            "198.18.0.1",
            "198.19.255.255",
            "64:ff9b::a00:1",
            "2002:a00:1::1",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}
