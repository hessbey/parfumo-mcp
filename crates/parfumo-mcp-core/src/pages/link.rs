//! The form that links, tests or unlinks a Parfumo account, and what its buttons do.
//!
//! One form with several submit buttons: the main one links the account with the Parfumo
//! login; the others (session cookie, test, unlink) sit under "More options". Nothing
//! typed here ever goes through the conversation with the assistant.

use serde::Deserialize;

use super::{Lang, esc, msg};
use crate::sessions::{SessionError, Sessions};

/// Where the form is served, which changes its wording and its first field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The `/link` page of the HTTP server: the user proves who they are with an access key.
    Server,
    /// The login page of the local stdio server: one user, on their own computer.
    Computer,
}

pub fn title(lang: Lang) -> &'static str {
    lang.pick("Link your Parfumo account", "Relier votre compte Parfumo", "Parfumo-Konto verknüpfen")
}

fn foot(lang: Lang) -> &'static str {
    lang.pick(
        "Unofficial tool, not affiliated with Parfumo. Nothing you type here is ever shown to your assistant.",
        "Outil non officiel, sans lien avec Parfumo. Rien de ce que vous saisissez ici n'est montré à votre assistant.",
        "Inoffizielles Tool ohne Verbindung zu Parfumo. Nichts, was du hier eingibst, wird deinem Assistenten gezeigt.",
    )
}

/// The form, posted to `action`. `message` is shown above it, and `more_open` keeps
/// "More options" unfolded after one of its buttons was used.
pub fn form(lang: Lang, place: Place, action: &str, message: &str, more_open: bool) -> String {
    let t = |en, fr, de| lang.pick(en, fr, de);
    let key = match place {
        Place::Server => format!(
            "<label for=\"k\">{}</label>\n<input id=\"k\" type=\"password\" name=\"access_key\" required placeholder=\"pmcp_…\">\n<p class=\"hint\">{}</p>\n",
            t("Access key", "Clé d'accès", "Zugangsschlüssel"),
            t(
                "Given to you by the person who runs this server.",
                "Fournie par la personne qui gère ce serveur.",
                "Du hast ihn von der Person erhalten, die diesen Server betreibt."
            ),
        ),
        Place::Computer => String::new(),
    };
    format!(
        r#"{message}
<form method="post" action="{action}" autocomplete="off">
<input type="hidden" name="lang" value="{lang}">
{key}<label for="pu">{nick}</label>
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
        action = esc(action),
        lang = lang.code(),
        open = if more_open { " open" } else { "" },
        nick = t("Parfumo nickname or e-mail", "Pseudo ou e-mail Parfumo", "Parfumo-Benutzername oder E-Mail"),
        password = t("Parfumo password", "Mot de passe Parfumo", "Parfumo-Passwort"),
        password_hint = match place {
            Place::Server => t(
                "Kept encrypted on this server, which logs in again by itself when the session expires.",
                "Conservé chiffré sur ce serveur, qui se reconnecte seul quand la session expire.",
                "Wird verschlüsselt auf diesem Server gespeichert, der sich nach Ablauf der Sitzung selbst wieder anmeldet."
            ),
            Place::Computer => t(
                "Kept encrypted on this computer and sent only to Parfumo. The program logs in again by itself when the session expires.",
                "Conservé chiffré sur cet ordinateur et envoyé uniquement à Parfumo. Le programme se reconnecte seul quand la session expire.",
                "Wird verschlüsselt auf diesem Computer gespeichert und nur an Parfumo gesendet. Das Programm meldet sich nach Ablauf der Sitzung selbst wieder an."
            ),
        },
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
        manage_hint = match place {
            Place::Server => t(
                "Check that Parfumo still recognises your account, or erase what this server stores about it. Only the access key is needed.",
                "Vérifiez que Parfumo reconnaît toujours votre compte, ou effacez ce que ce serveur en conserve. Seule la clé d'accès est nécessaire.",
                "Prüfe, ob Parfumo dein Konto noch erkennt, oder lösche, was dieser Server darüber speichert. Dafür genügt der Zugangsschlüssel."
            ),
            Place::Computer => t(
                "Check that Parfumo still recognises your account, or erase what this computer stores about it.",
                "Vérifiez que Parfumo reconnaît toujours votre compte, ou effacez ce que cet ordinateur en conserve.",
                "Prüfe, ob Parfumo dein Konto noch erkennt, oder lösche, was dieser Computer darüber speichert."
            ),
        },
        test = t("Test the link", "Tester la liaison", "Verknüpfung testen"),
        unlink = t("Unlink my account", "Délier mon compte", "Verknüpfung aufheben"),
        foot = foot(lang),
    )
}

/// What is left of the page once nothing more is to be typed: `message` and the footer.
pub fn done(lang: Lang, message: &str) -> String {
    format!("{message}\n<p class=\"foot\">{}</p>", foot(lang))
}

/// The fields of the form. `access_key` is only read by the HTTP server.
#[derive(Deserialize)]
pub struct LinkForm {
    #[serde(default)]
    pub access_key: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub parfumo_username: String,
    #[serde(default)]
    pub parfumo_password: String,
    #[serde(default)]
    pub cookie: String,
    #[serde(default)]
    pub accept_risk: String,
    #[serde(default)]
    pub lang: Option<String>,
}

impl LinkForm {
    /// Whether the button used sits under "More options".
    pub fn more_open(&self) -> bool {
        matches!(self.action.as_str(), "cookie" | "status" | "unlink")
    }
}

/// What a button did, as the page reports it.
#[derive(Debug)]
pub struct Outcome {
    /// HTTP status of the answer.
    pub status: u16,
    pub ok: bool,
    /// The message, in the language of the page, not yet escaped.
    pub text: String,
    /// The account is now linked and Parfumo recognises its session.
    pub linked: bool,
}

impl Outcome {
    fn refused(text: &str) -> Self {
        Self { status: 400, ok: false, text: text.to_string(), linked: false }
    }

    /// The message as HTML, to show above the form.
    pub fn message(&self) -> String {
        msg(self.ok, &self.text)
    }
}

fn session_error_text(lang: Lang, place: Place, e: &SessionError) -> String {
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
        SessionError::CaptchaRequired => match place {
            Place::Server => lang.pick(
                "Parfumo asks for a human check, which this server never solves. Log in with your browser, then use a session cookie (More options).",
                "Parfumo demande une vérification humaine, que ce serveur ne résout jamais. Connectez-vous dans votre navigateur, puis utilisez un cookie de session (Plus d'options).",
                "Parfumo verlangt eine Prüfung durch einen Menschen, die dieser Server nie löst. Melde dich im Browser an und verwende dann ein Sitzungs-Cookie (Weitere Optionen).",
            ),
            Place::Computer => lang.pick(
                "Parfumo asks for a human check, which this program never solves. Log in with your browser, then use a session cookie (More options).",
                "Parfumo demande une vérification humaine, que ce programme ne résout jamais. Connectez-vous dans votre navigateur, puis utilisez un cookie de session (Plus d'options).",
                "Parfumo verlangt eine Prüfung durch einen Menschen, die dieses Programm nie löst. Melde dich im Browser an und verwende dann ein Sitzungs-Cookie (Weitere Optionen).",
            ),
        }
        .into(),
        SessionError::Internal(m) => format!("{} {m}", lang.pick("Error:", "Erreur :", "Fehler:")),
    }
}

/// Does what the button of `f` asks for the Parfumo link of `user_id`. The caller has
/// already checked where the request comes from and who the user is.
pub async fn perform(sessions: &Sessions, user_id: i64, lang: Lang, place: Place, f: &LinkForm) -> Outcome {
    let t = |en, fr, de| lang.pick(en, fr, de);
    if matches!(f.action.as_str(), "credentials" | "cookie") && f.accept_risk != "yes" {
        return Outcome::refused(t(
            "Tick the box about the risk of suspension to link an account.",
            "Cochez la case sur le risque de suspension pour relier un compte.",
            "Setze das Häkchen zum Sperrrisiko, um ein Konto zu verknüpfen.",
        ));
    }
    // The text of a success, and whether the account is linked and recognised.
    let outcome: Result<(&str, bool), SessionError> = match f.action.as_str() {
        "credentials" => {
            if f.parfumo_username.trim().is_empty() || f.parfumo_password.is_empty() {
                return Outcome::refused(t(
                    "Enter your Parfumo nickname and password.",
                    "Saisissez votre pseudo et votre mot de passe Parfumo.",
                    "Gib deinen Parfumo-Benutzernamen und dein Passwort ein.",
                ));
            }
            sessions.link_credentials(user_id, f.parfumo_username.trim(), &f.parfumo_password).await.map(|()| {
                (
                    t(
                        "Your Parfumo account is linked. You can close this page.",
                        "Votre compte Parfumo est relié. Vous pouvez fermer cette page.",
                        "Dein Parfumo-Konto ist verknüpft. Du kannst diese Seite schließen.",
                    ),
                    true,
                )
            })
        }
        "cookie" => {
            if f.cookie.trim().is_empty() {
                return Outcome::refused(t("Paste the Cookie header.", "Collez l'en-tête Cookie.", "Füge den Cookie-Header ein."));
            }
            sessions.link_cookie(user_id, f.cookie.trim()).await.map(|logged_in| {
                if logged_in {
                    (
                        t(
                            "Your Parfumo account is linked: Parfumo recognises the session. You can close this page.",
                            "Votre compte Parfumo est relié : Parfumo reconnaît la session. Vous pouvez fermer cette page.",
                            "Dein Parfumo-Konto ist verknüpft: Parfumo erkennt die Sitzung. Du kannst diese Seite schließen.",
                        ),
                        true,
                    )
                } else {
                    (
                        t(
                            "The cookie is stored, but Parfumo does not recognise it as a logged-in session. Check that it comes from a browser logged in to parfumo.com.",
                            "Le cookie est enregistré, mais Parfumo ne le reconnaît pas comme une session connectée. Vérifiez qu'il vient d'un navigateur connecté à parfumo.com.",
                            "Das Cookie ist gespeichert, aber Parfumo erkennt es nicht als angemeldete Sitzung. Prüfe, ob es aus einem bei parfumo.com angemeldeten Browser stammt.",
                        ),
                        false,
                    )
                }
            })
        }
        "status" => match sessions.link_info(user_id) {
            Ok(None) => Err(SessionError::NotLinked),
            Ok(Some(_)) => sessions.ensure_logged_in(user_id).await.map(|_| {
                (
                    t(
                        "The link works: Parfumo recognises your account.",
                        "La liaison fonctionne : Parfumo reconnaît votre compte.",
                        "Die Verknüpfung funktioniert: Parfumo erkennt dein Konto.",
                    ),
                    false,
                )
            }),
            Err(e) => Err(e),
        },
        "unlink" => sessions.unlink(user_id).map(|was| {
            let text = if was {
                t(
                    "Your account is unlinked: the stored credentials and cookies are erased.",
                    "Votre compte est délié : les identifiants et cookies enregistrés sont effacés.",
                    "Die Verknüpfung ist aufgehoben: gespeicherte Zugangsdaten und Cookies sind gelöscht.",
                )
            } else {
                t("No account was linked.", "Aucun compte n'était relié.", "Es war kein Konto verknüpft.")
            };
            (text, false)
        }),
        _ => return Outcome::refused(t("Unknown action.", "Action inconnue.", "Unbekannte Aktion.")),
    };
    match outcome {
        Ok((text, linked)) => Outcome { status: 200, ok: true, text: text.to_string(), linked },
        Err(e) => Outcome { status: 200, ok: false, text: session_error_text(lang, place, &e), linked: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_form_asks_for_no_access_key_and_speaks_of_this_computer() {
        for lang in Lang::ALL {
            let server = form(lang, Place::Server, "/link", "", false);
            let local = form(lang, Place::Computer, "/secret", "", false);
            assert!(server.contains("name=\"access_key\"") && server.contains("action=\"/link\""), "{server}");
            assert!(!local.contains("access_key") && !local.contains("pmcp_") && local.contains("action=\"/secret\""), "{local}");
            for field in ["parfumo_username", "parfumo_password", "accept_risk", "value=\"credentials\"", "value=\"unlink\""] {
                assert!(server.contains(field) && local.contains(field), "{field} in {}", lang.code());
            }
            let words = lang.pick("this server", "ce serveur", "Server");
            assert!(server.contains(words) && !local.contains(words), "{} in {}", words, lang.code());
        }
        assert!(form(Lang::En, Place::Computer, "/x", "", true).contains("<details open>"));
        assert!(!done(Lang::En, "<p class=\"msg\">ok</p>").contains("<form"));
    }
}
