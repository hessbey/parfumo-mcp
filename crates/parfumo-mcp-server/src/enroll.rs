//! Enrolment page `/link`: each user links, tests or unlinks their own Parfumo account.
//! Nothing here is ever typed into the assistant conversation or passed as a tool argument.
//!
//! The form and what its buttons do are shared with the local stdio server
//! (`parfumo_mcp_core::pages::link`); this page adds the access key that tells who the
//! user is.

use axum::{
    Form,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::Deserialize;

use parfumo_mcp_core::pages::{
    Lang,
    link::{self, LinkForm, Place},
    msg,
};

use crate::{
    i18n,
    web::{LoginError, ServerState, client_ip, origin_ok, origin_refused, page, verify_key},
};

/// The page, with `message` above the form.
fn show(status: StatusCode, lang: Lang, message: &str, more_open: bool) -> Response {
    page(status, lang, link::title(lang), &switcher(lang), &link::form(lang, Place::Server, "/link", message, more_open))
}

#[derive(Deserialize)]
pub struct LangQuery {
    #[serde(default)]
    lang: Option<String>,
}

pub async fn get(Query(q): Query<LangQuery>, headers: HeaderMap) -> Response {
    show(StatusCode::OK, i18n::choose(q.lang.as_deref(), &headers), "", false)
}

/// Links to the same page in the other languages.
fn switcher(current: Lang) -> String {
    crate::web::lang_switcher(current, |lang| format!("/link?lang={}", lang.code()))
}

pub async fn post(State(st): State<ServerState>, headers: HeaderMap, Form(f): Form<LinkForm>) -> Response {
    let lang = i18n::choose(f.lang.as_deref(), &headers);
    let refuse = |status: StatusCode, text: &str| show(status, lang, &msg(false, text), f.more_open());
    if !origin_ok(&st.http, &headers) {
        return refuse(StatusCode::FORBIDDEN, &origin_refused(&st.http, lang, "/link"));
    }
    let user = match verify_key(&st.core.db, &f.access_key, client_ip(&headers).as_deref()) {
        Ok(u) => u,
        Err(LoginError::Locked) => return refuse(StatusCode::TOO_MANY_REQUESTS, crate::web::too_many_failures(lang)),
        Err(LoginError::Invalid) => return refuse(StatusCode::UNAUTHORIZED, crate::web::wrong_key(lang)),
    };
    let outcome = link::perform(&st.core.sessions, user.id, lang, Place::Server, &f).await;
    tracing::info!(user = %user.name, action = %f.action, ok = outcome.ok, "enrolment action");
    show(StatusCode::from_u16(outcome.status).unwrap_or(StatusCode::OK), lang, &outcome.message(), f.more_open())
}
