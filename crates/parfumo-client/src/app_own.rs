//! The member's own nickname, ratings and classification votes of a perfume through the
//! application interface (`docs/APP_API.md`): the same models as the web panels fill.
//!
//! The tools decide from these reads which publicly visible votes to send, so what the
//! recorded answers do not prove is left to the web panels:
//! - a rating of `0`: the application shows it as no rating, where the web panel tells a
//!   rating of zero from a scale not rated;
//! - a `voted` flag other than `0` and a classification that is `locked`: only an answer
//!   without any vote was recorded;
//! - a call answered 404: what the interface says of a perfume it does not know was not
//!   recorded, and the web panel words its own refusal.
//!
//! One reading goes beyond the recordings, which hold a perfume rated on its five scales and
//! perfumes not rated at all: a scale the answer does not name, or names without a value, is
//! read as not rated. The empty list of a perfume not rated points to it and the application
//! reads it so; whatever else a scale may hold that is not a rating goes to the web panel.

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    app_db::{count, number, text},
    models::{ClassChoice, MemberRatings},
    site::routes,
};

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// The answer of a call, or `None` when the interface has nothing under that address.
fn served(answer: Result<Value, ClientError>) -> Result<Option<Value>, ClientError> {
    match answer {
        Ok(answer) => Ok(Some(answer)),
        Err(ClientError::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

impl ParfumoClient {
    /// The nickname of the member (`GET /auth/me`), kept for the session as the web code
    /// keeps it. `None` for a nickname that could not stand in the address of the member's
    /// pages, which the web code refuses too.
    pub(crate) async fn app_own_nick(&self) -> Result<Option<String>, ClientError> {
        let nick = self.app_nick().await?;
        if nick.contains(['/', '?', '#', ' ']) {
            return Ok(None);
        }
        *self.t.nick.lock().unwrap_or_else(|e| e.into_inner()) = Some(nick.clone());
        Ok(Some(nick))
    }

    /// The member's five ratings of a perfume (`GET /rating`), out of 10 as the web panel
    /// gives them: the interface counts them out of 100. A perfume not rated answers an
    /// empty list; a scale the answer does not name, or names without a value, is taken as
    /// not rated (never recorded: see the module). `None` when the answer is not the
    /// member's, or holds a value that is not a rating above 0.
    pub(crate) async fn app_my_ratings_of(&self, p_id: u64) -> Result<Option<MemberRatings>, ClientError> {
        let Some(answer) = served(self.app_get("/rating", &[pair("p_id", p_id)]).await)? else { return Ok(None) };
        // Answered as to a visitor: an empty list would not mean "not rated".
        if number(&answer, "login_status") != Some(1.0) {
            return Ok(None);
        }
        let rated = match answer.get("ratings") {
            Some(Value::Array(none)) if none.is_empty() => return Ok(Some(MemberRatings::default())),
            Some(rated @ Value::Object(scales)) => {
                // A scale under a name the model does not know would read as not rated.
                if scales.keys().any(|k| !routes::RATING_SCALES.contains(&k.as_str())) {
                    return Ok(None);
                }
                rated
            }
            _ => return Err(ClientError::ParserOutdated("/app/v1/rating: no ratings".into())),
        };
        let mut out = MemberRatings::default();
        for scale in routes::RATING_SCALES {
            match rated.get(*scale) {
                None | Some(Value::Null) => {}
                // No rounding: the site holds a rating in tenths, and the tools compare it as such.
                Some(_) => match number(rated, scale) {
                    Some(percent) if percent > 0.0 && percent <= 100.0 => out.set(scale, Some(percent as f32 / 10.0)),
                    _ => return Ok(None),
                },
            }
        }
        Ok(Some(out))
    }

    /// The classification form of a perfume (`GET /classification`): every label with the
    /// identifiers the vote of the web site takes (`c` and `ct`, the same numbers as on the
    /// web panel), in the order of the web panel. `None` unless the answer is the open form
    /// without any vote of the member, the only one whose meaning was recorded.
    pub(crate) async fn app_my_classification_of(&self, p_id: u64) -> Result<Option<Vec<ClassChoice>>, ClientError> {
        let Some(answer) = served(self.app_get("/classification", &[pair("p_id", p_id)]).await)? else { return Ok(None) };
        let outdated = |what: &str| ClientError::ParserOutdated(format!("/app/v1/classification: {what}"));
        if text(&answer, "status").as_deref() != Some("success") || number(&answer, "locked") != Some(0.0) {
            return Ok(None);
        }
        let groups = answer.get("groups").and_then(Value::as_array).ok_or_else(|| outdated("no groups"))?;
        let mut out = Vec::new();
        // The web panel shows the groups the other way round (`Type` first) and the labels of
        // a group in the same order: the tools list them alike whichever side answered.
        for group in groups.iter().rev() {
            let (Some(group_id), Some(name)) = (count(group, "c"), text(group, "name")) else {
                return Err(outdated("group without id or title"));
            };
            for option in group.get("options").and_then(Value::as_array).ok_or_else(|| outdated("group without options"))? {
                let (Some(label_id), Some(label)) = (count(option, "ct"), text(option, "name")) else {
                    return Err(outdated("option without id or label"));
                };
                // `0` is the flag of a label not voted for; no other value was ever seen.
                if number(option, "voted") != Some(0.0) {
                    return Ok(None);
                }
                out.push(ClassChoice { group: name.clone(), label, voted: false, group_id, label_id });
            }
        }
        if out.is_empty() {
            return Err(outdated("no label found"));
        }
        Ok(Some(out))
    }
}
