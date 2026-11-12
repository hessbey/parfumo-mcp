//! Enrolment page `/link`: each user links, tests or unlinks their own Parfumo account.
//! Nothing here is ever typed into the assistant conversation or passed as a tool argument.
//!
//! The page is one form with several submit buttons: the main one links the account with
//! the Parfumo login; the others (session cookie, test, unlink) sit under "More options".

use axum::{
    Form,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::Deserialize;

use parfumo_mcp_core::sessions::SessionError;

use crate::{
    i18n::Lang,
    web::{LoginError, ServerState, client_ip, esc, origin_ok, origin_refused, page, verify_key},
};

fn title(lang: Lang) -> &'static str {
    lang.pick("Link your Parfumo account", "Relier votre compte Parfumo", "Parfumo-Konto verknüpfen")
}

/// The form. `more_open` keeps "More options" unfolded after one of its buttons was used.
fn form(lang: Lang, message: &str, more_open: bool) -> String {
    let t = |en, fr, de| lang.pick(en, fr, de);
    format!(
        r#"{message}
<form method="post" action="/link" autocomplete="off">
<input type="hidden" name="lang" value="{lang}">
<label for="k">{key}</label>
<input id="k" type="password" name="access_key" required placeholder="pmcp_…">
<p class="hint">{key_hint}</p>
<label for="pu">{nick}</label>
<input id="pu" type="text" name="parfumo_username">
<label for="pp">{password}</label>
<input id="pp" type="password" name="parfumo_password">
<p class="hint">{password_hint}</p>
<label class="check"><input type="checkbox" name="accept_risk" value="yes"><span>{risk}</span></label>
<button type="submit" name="action" value="credentials">{link}</button>
<details{open}>
<summary>{more}</summary>
<h2>{cookie_title}</h2>
<p class="hint">{cookie_hint}</p>
<textarea name="cookie" rows="2" placeholder="pf_auth=…" aria-label="Cookie"></textarea>
<button type="submit" class="quiet" name="action" value="cookie">{cookie_link}</button>
<h2>{manage_title}</h2>
<p class="hint">{manage_hint}</p>
<div class="pair">
<button type="submit" class="quiet" name="action" value="status">{test}</button>
<button type="submit" class="quiet" name="action" value="unlink">{unlink}</button>
</div>
</details>
</form>
<p class="foot">{foot}</p>"#,
        lang = lang.code(),
        open = if more_open { " open" } else { "" },
        key = t("Access key", "Clé d'accès", "Zugangsschlüssel"),
        key_hint = t(
            "Given to you by the person who runs this server.",
            "Fournie par la personne qui gère ce serveur.",
            "Du hast ihn von der Person erhalten, die diesen Server betreibt."
        ),
        nick = t("Parfumo nickname or e-mail", "Pseudo ou e-mail Parfumo", "Parfumo-Benutzername oder E-Mail"),
        password = t("Parfumo password", "Mot de passe Parfumo", "Parfumo-Passwort"),
        password_hint = t(
            "Kept encrypted on this server, which logs in again by itself when the session expires.",
            "Conservé chiffré sur ce serveur, qui se reconnecte seul quand la session expire.",
            "Wird verschlüsselt auf diesem Server gespeichert, der sich nach Ablauf der Sitzung selbst wieder anmeldet."
        ),
        risk = t(
            "I understand that Parfumo may suspend my account because of automated access.",
            "J'ai compris que Parfumo peut suspendre mon compte en raison d'un accès automatisé.",
            "Mir ist bewusst, dass Parfumo mein Konto wegen automatisierter Zugriffe sperren kann."
        ),
        link = t("Link my account", "Relier mon compte", "Konto verknüpfen"),
        more = t("More options", "Plus d'options", "Weitere Optionen"),
        cookie_title = t(
            "Use a session cookie instead of my password",
            "Utiliser un cookie de session au lieu du mot de passe",
            "Sitzungs-Cookie statt Passwort verwenden"
        ),
        cookie_hint = t(
            "In a browser logged in to parfumo.com: developer tools, Network tab, any request to www.parfumo.com, request header \"Cookie\". Tick the box above as well.",
            "Dans un navigateur connecté à parfumo.com : outils de développement, onglet Réseau, une requête vers www.parfumo.com, en-tête de requête « Cookie ». Cochez aussi la case ci-dessus.",
            "In einem bei parfumo.com angemeldeten Browser: Entwicklerwerkzeuge, Tab „Netzwerk“, eine Anfrage an www.parfumo.com, Anfrage-Header „Cookie“. Setze auch das Häkchen oben."
        ),
        cookie_link = t("Link with this cookie", "Relier avec ce cookie", "Mit diesem Cookie verknüpfen"),
        manage_title = t("Already linked?", "Déjà relié ?", "Bereits verknüpft?"),
        manage_hint = t(
            "Check that Parfumo still recognises your account, or erase what this server stores about it. Only the access key is needed.",
            "Vérifiez que Parfumo reconnaît toujours votre compte, ou effacez ce que ce serveur en conserve. Seule la clé d'accès est nécessaire.",
            "Prüfe, ob Parfumo dein Konto noch erkennt, oder lösche, was dieser Server darüber speichert. Dafür genügt der Zugangsschlüssel."
        ),
        test = t("Test the link", "Tester la liaison", "Verknüpfung testen"),
        unlink = t("Unlink my account", "Délier mon compte", "Verknüpfung aufheben"),
        foot = t(
            "Unofficial tool, not affiliated with Parfumo. Nothing you type here is ever shown to your assistant.",
            "Outil non officiel, sans lien avec Parfumo. Rien de ce que vous saisissez ici n'est montré à votre assistant.",
            "Inoffizielles Tool ohne Verbindung zu Parfumo. Nichts, was du hier eingibst, wird deinem Assistenten gezeigt."
        ),
    )
}

fn msg(ok: bool, text: &str) -> String {
    format!("<p class=\"msg{}\">{}</p>", if ok { "" } else { " err" }, esc(text))
}

#[derive(Deserialize)]
pub struct LangQuery {
    #[serde(default)]
    lang: Option<String>,
}

pub async fn get(Query(q): Query<LangQuery>, headers: HeaderMap) -> Response {
    let lang = Lang::choose(q.lang.as_deref(), &headers);
    page(StatusCode::OK, lang, title(lang), &switcher(lang), &form(lang, "", false))
}

/// Links to the same page in the other languages.
fn switcher(current: Lang) -> String {
    crate::web::lang_switcher(current, |lang| format!("/link?lang={}", lang.code()))
}

#[derive(Deserialize)]
pub struct LinkForm {
    #[serde(default)]
    access_key: String,
    action: String,
    #[serde(default)]
    parfumo_username: String,
    #[serde(default)]
    parfumo_password: String,
    #[serde(default)]
    cookie: String,
    #[serde(default)]
    accept_risk: String,
    #[serde(default)]
    lang: Option<String>,
}

fn session_error_text(lang: Lang, e: &SessionError) -> String {
    match e {
        SessionError::NotLinked => {
            lang.pick("No Parfumo account is linked.", "Aucun compte Parfumo n'est relié.", "Es ist kein Parfumo-Konto verknüpft.").into()
        }
        SessionError::ReauthRequired => lang
            .pick(
                "Parfumo refused the login: wrong nickname or password, or three attempts without success. Nothing new was stored.",
                "Parfumo a refusé la connexion : pseudo ou mot de passe incorrect, ou trois tentatives sans succès. Rien de nouveau n'a été enregistré.",
                "Parfumo hat die Anmeldung abgelehnt: falscher Benutzername oder falsches Passwort, oder drei erfolglose Versuche. Es wurde nichts Neues gespeichert.",
            )
            .into(),
        SessionError::CaptchaRequired => lang
            .pick(
                "Parfumo asks for a human check, which this server never solves. Log in with your browser, then use a session cookie (More options).",
                "Parfumo demande une vérification humaine, que ce serveur ne résout jamais. Connectez-vous dans votre navigateur, puis utilisez un cookie de session (Plus d'options).",
                "Parfumo verlangt eine Prüfung durch einen Menschen, die dieser Server nie löst. Melde dich im Browser an und verwende dann ein Sitzungs-Cookie (Weitere Optionen).",
            )
            .into(),
        SessionError::Internal(m) => format!("{} {m}", lang.pick("Error:", "Erreur :", "Fehler:")),
    }
}

pub async fn post(State(st): State<ServerState>, headers: HeaderMap, Form(f): Form<LinkForm>) -> Response {
    let lang = Lang::choose(f.lang.as_deref(), &headers);
    let more_open = matches!(f.action.as_str(), "cookie" | "status" | "unlink");
    let reply = |status: StatusCode, ok: bool, text: &str| page(status, lang, title(lang), &switcher(lang), &form(lang, &msg(ok, text), more_open));
    let t = |en, fr, de| lang.pick(en, fr, de);
    if !origin_ok(&st.http, &headers) {
        return reply(StatusCode::FORBIDDEN, false, &origin_refused(&st.http, lang, "/link"));
    }
    let user = match verify_key(&st.core.db, &f.access_key, client_ip(&headers).as_deref()) {
        Ok(u) => u,
        Err(LoginError::Locked) => return reply(StatusCode::TOO_MANY_REQUESTS, false, crate::web::too_many_failures(lang)),
        Err(LoginError::Invalid) => return reply(StatusCode::UNAUTHORIZED, false, crate::web::wrong_key(lang)),
    };
    let needs_risk = matches!(f.action.as_str(), "credentials" | "cookie");
    if needs_risk && f.accept_risk != "yes" {
        return reply(
            StatusCode::BAD_REQUEST,
            false,
            t(
                "Tick the box about the risk of suspension to link an account.",
                "Cochez la case sur le risque de suspension pour relier un compte.",
                "Setze das Häkchen zum Sperrrisiko, um ein Konto zu verknüpfen.",
            ),
        );
    }
    let outcome: Result<String, SessionError> = match f.action.as_str() {
        "credentials" => {
            if f.parfumo_username.trim().is_empty() || f.parfumo_password.is_empty() {
                return reply(
                    StatusCode::BAD_REQUEST,
                    false,
                    t(
                        "Enter your Parfumo nickname and password.",
                        "Saisissez votre pseudo et votre mot de passe Parfumo.",
                        "Gib deinen Parfumo-Benutzernamen und dein Passwort ein.",
                    ),
                );
            }
            st.core.sessions.link_credentials(user.id, f.parfumo_username.trim(), &f.parfumo_password).await.map(|()| {
                t(
                    "Your Parfumo account is linked. You can close this page.",
                    "Votre compte Parfumo est relié. Vous pouvez fermer cette page.",
                    "Dein Parfumo-Konto ist verknüpft. Du kannst diese Seite schließen.",
                )
                .to_string()
            })
        }
        "cookie" => {
            if f.cookie.trim().is_empty() {
                return reply(
                    StatusCode::BAD_REQUEST,
                    false,
                    t("Paste the Cookie header.", "Collez l'en-tête Cookie.", "Füge den Cookie-Header ein."),
                );
            }
            st.core.sessions.link_cookie(user.id, f.cookie.trim()).await.map(|logged_in| {
                if logged_in {
                    t(
                        "Your Parfumo account is linked: Parfumo recognises the session. You can close this page.",
                        "Votre compte Parfumo est relié : Parfumo reconnaît la session. Vous pouvez fermer cette page.",
                        "Dein Parfumo-Konto ist verknüpft: Parfumo erkennt die Sitzung. Du kannst diese Seite schließen.",
                    )
                } else {
                    t(
                        "The cookie is stored, but Parfumo does not recognise it as a logged-in session. Check that it comes from a browser logged in to parfumo.com.",
                        "Le cookie est enregistré, mais Parfumo ne le reconnaît pas comme une session connectée. Vérifiez qu'il vient d'un navigateur connecté à parfumo.com.",
                        "Das Cookie ist gespeichert, aber Parfumo erkennt es nicht als angemeldete Sitzung. Prüfe, ob es aus einem bei parfumo.com angemeldeten Browser stammt.",
                    )
                }
                .to_string()
            })
        }
        "status" => match st.core.sessions.link_info(user.id) {
            Ok(None) => Err(SessionError::NotLinked),
            Ok(Some(_)) => st.core.sessions.ensure_logged_in(user.id).await.map(|_| {
                t(
                    "The link works: Parfumo recognises your account.",
                    "La liaison fonctionne : Parfumo reconnaît votre compte.",
                    "Die Verknüpfung funktioniert: Parfumo erkennt dein Konto.",
                )
                .to_string()
            }),
            Err(e) => Err(e),
        },
        "unlink" => st.core.sessions.unlink(user.id).map(|was| {
            if was {
                t(
                    "Your account is unlinked: the stored credentials and cookies are erased.",
                    "Votre compte est délié : les identifiants et cookies enregistrés sont effacés.",
                    "Die Verknüpfung ist aufgehoben: gespeicherte Zugangsdaten und Cookies sind gelöscht.",
                )
            } else {
                t("No account was linked.", "Aucun compte n'était relié.", "Es war kein Konto verknüpft.")
            }
            .to_string()
        }),
        _ => return reply(StatusCode::BAD_REQUEST, false, t("Unknown action.", "Action inconnue.", "Unbekannte Aktion.")),
    };
    tracing::info!(user = %user.name, action = %f.action, ok = outcome.is_ok(), "enrolment action");
    match outcome {
        Ok(text) => reply(StatusCode::OK, true, &text),
        Err(e) => reply(StatusCode::OK, false, &session_error_text(lang, &e)),
    }
}
