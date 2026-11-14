//! Account side of the client (jalon 2): the member's own collection and wear history.
//!
//! Every route was confirmed logged in at jalon 0 bis (DISCOVERY §3.2, §3.3, §5). Reads go
//! through ShelfControl, whose page embeds a whole list as JSON. Writes are described by
//! [`WriteCall`] values built by the functions of [`calls`]: the same value is either sent
//! by [`ParfumoClient::execute`] or shown as is by a tool's `dry_run`.

use crate::{
    client::ParfumoClient,
    error::ClientError,
    models::*,
    parse,
    site::{markers, routes},
    transport::empty_body,
};

/// Builders of the write requests. Nothing is sent here.
pub mod calls {
    use super::*;

    fn call(path: &str, expect: Expect, form: &[(&str, String)]) -> WriteCall {
        WriteCall { method: "POST".into(), path: path.to_string(), form: form.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(), expect }
    }

    /// Adds a perfume to a list. Answer: the new entry id; empty when already there; `0`
    /// for an unknown perfume.
    pub fn collection_add(p_id: u64, list_id: u64) -> WriteCall {
        call(
            routes::WARDROBE_ADD_REMOVE,
            Expect::Anything,
            &[("p", p_id.to_string()), ("t", list_id.to_string()), ("action", "true".into()), ("hidden", "No".into())],
        )
    }

    /// Removes a perfume from a list (empty answer). From list 1 the site moves it to list 2.
    pub fn collection_remove(p_id: u64, list_id: u64) -> WriteCall {
        call(
            routes::WARDROBE_ADD_REMOVE,
            Expect::Anything,
            &[("p", p_id.to_string()), ("t", list_id.to_string()), ("action", "false".into()), ("hidden", "No".into())],
        )
    }

    pub fn collection_copy_move(entry_id: u64, p_id: u64, from_list: u64, to_list: u64, copy: bool) -> WriteCall {
        call(
            routes::WARDROBE_COPY_MOVE,
            Expect::One,
            &[
                ("wr_id", entry_id.to_string()),
                ("list_id", from_list.to_string()),
                ("new_list_id", to_list.to_string()),
                ("p_id", p_id.to_string()),
                ("copy_move", if copy { "copy" } else { "move" }.into()),
            ],
        )
    }

    /// The four fields are always sent together (the form of the site does the same).
    pub fn collection_details(entry_id: u64, quantity: u32, bottle_size: &str, paid: &str, currency: &str) -> WriteCall {
        call(
            routes::WARDROBE_UPDATE_DETAILS,
            Expect::StatusOk,
            &[
                ("id", entry_id.to_string()),
                ("quantity", quantity.to_string()),
                ("bottle_size", bottle_size.into()),
                ("paid", paid.into()),
                ("currency", currency.into()),
            ],
        )
    }

    pub fn collection_remaining_ml(entry_id: u64, value: &str) -> WriteCall {
        call(routes::SHELF_COLUMNS, Expect::OkTrue, &[("op", "ml".into()), ("row", entry_id.to_string()), ("value", value.into())])
    }

    pub fn collection_custom_value(entry_id: u64, column_id: u64, value: &str) -> WriteCall {
        call(
            routes::SHELF_COLUMNS,
            Expect::OkTrue,
            &[("op", "value".into()), ("row", entry_id.to_string()), ("id", column_id.to_string()), ("value", value.into())],
        )
    }

    /// Sets the private note of an entry; an empty text deletes it.
    pub fn collection_note(entry_id: u64, text: &str) -> WriteCall {
        if text.trim().is_empty() {
            call(routes::MEMO_DELETE, Expect::Anything, &[("type", "wardrobe".into()), ("id", entry_id.to_string())])
        } else {
            call(routes::MEMO_CREATE, Expect::StatusOk, &[("type", "wardrobe".into()), ("id", entry_id.to_string()), ("text", text.into())])
        }
    }

    pub fn list_create() -> WriteCall {
        call(routes::WARDROBE_ADD_LIST, Expect::StatusOk, &[])
    }

    /// `list` is the numeric id, or a placeholder in a dry run of a creation.
    pub fn list_rename(list: &str, name: &str, notes: &str) -> WriteCall {
        call(routes::WARDROBE_SET_NAME, Expect::Body, &[("id", list.into()), ("name", name.into()), ("notes", notes.into())])
    }

    /// Toggles the hidden state; the answer is the new state (`Yes` = hidden).
    pub fn list_toggle_hidden(list_id: u64) -> WriteCall {
        call(routes::WARDROBE_LOCK_TOGGLE, Expect::Body, &[("id", list_id.to_string())])
    }

    pub fn list_delete(list_id: u64) -> WriteCall {
        call(routes::WARDROBE_DELETE_LIST, Expect::Anything, &[("id", list_id.to_string())])
    }

    /// Logs a wear now. Publicly visible: the perfume shows on the member's profile.
    pub fn wear_log(p_id: u64) -> WriteCall {
        call(routes::WEAR_LOG, Expect::Body, &[("p", p_id.to_string())])
    }

    /// Clears the "currently wearing" display; the history entry stays.
    pub fn wear_clear_current() -> WriteCall {
        call(routes::WEAR_LOG, Expect::Body, &[("p", "clear".into())])
    }

    /// `wear` is the numeric id, or a placeholder in a dry run. `when` is `YYYY-MM-DDTHH:MM`.
    pub fn wear_set_date(wear: &str, when: &str) -> WriteCall {
        call(routes::WEAR_SET_DATE, Expect::One, &[("id", wear.into()), ("new_date", when.into())])
    }

    pub fn wear_set_comment(wear: &str, comment: &str) -> WriteCall {
        call(routes::WEAR_SET_COMMENT, Expect::One, &[("id", wear.into()), ("comment", comment.into())])
    }

    pub fn wear_delete(wear_id: u64) -> WriteCall {
        call(routes::WEAR_DELETE, Expect::Anything, &[("id", wear_id.to_string())])
    }

    /// Rates a perfume on one scale. `tenths` is the rating out of 10 times ten (0-100).
    /// Publicly visible: it counts in the community average and shows among the voters.
    pub fn vote_add(p_id: u64, scale: &str, tenths: u32) -> WriteCall {
        call(routes::VOTE_ADD, Expect::StatusOk, &[("type", scale.into()), ("p_id", p_id.to_string()), ("v", tenths.to_string())])
    }

    pub fn vote_remove(p_id: u64, scale: &str) -> WriteCall {
        call(routes::VOTE_REMOVE, Expect::StatusOk, &[("type", scale.into()), ("p_id", p_id.to_string())])
    }

    /// Adds or removes one classification vote. Answer: `1` after an add, `0` after a removal.
    pub fn classification_vote(p_id: u64, group_id: u64, label_id: u64, add: bool) -> WriteCall {
        call(
            routes::CLASSIFY_TOGGLE,
            Expect::Body,
            &[
                ("p", p_id.to_string()),
                ("c", group_id.to_string()),
                ("ct", label_id.to_string()),
                ("action", if add { "add" } else { "remove" }.into()),
            ],
        )
    }

    /// Publishes the member's review of a perfume, or replaces the one they already wrote
    /// (the site's form posts the same fields in both cases). Seen in the script, never sent.
    pub fn review_save(p_id: u64, title: &str, text: &str) -> WriteCall {
        call(
            routes::REVIEW_CREATE,
            Expect::Accepted,
            &[("id", p_id.to_string()), ("title", title.into()), ("text", text.into()), ("yt", String::new()), ("paste_html", String::new())],
        )
    }

    /// Publishes or replaces the member's statement on a perfume. Seen in the script, never sent.
    pub fn statement_save(p_id: u64, text: &str) -> WriteCall {
        call(routes::STATEMENT_CREATE, Expect::Accepted, &[("id", p_id.to_string()), ("text", text.into())])
    }

    /// Deletes a review or statement. `h` is the token of the delete button, or a
    /// placeholder when the call is only shown. Seen in the script, never sent.
    pub fn text_delete(kind: TextKind, id: u64, p_id: u64, h: &str) -> WriteCall {
        let path = match kind {
            TextKind::Review => routes::REVIEW_DELETE,
            TextKind::Statement => routes::STATEMENT_DELETE,
        };
        call(path, Expect::Anything, &[("id", id.to_string()), ("p", p_id.to_string()), ("h", h.into())])
    }
}

/// Short, single-line excerpt of an unexpected answer, safe to put in an error message
/// (answers of these routes carry no credential: status words, ids, refusal texts).
fn excerpt(body: &str) -> String {
    let flat: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut = flat.char_indices().nth(160).map(|(i, _)| i).unwrap_or(flat.len());
    flat[..cut].to_string()
}

/// A refusal the site states in its answer: reported to the caller as such.
fn site_refusal(message: &serde_json::Value) -> ClientError {
    let msg = message.as_str().map(excerpt).filter(|m| !m.is_empty()).unwrap_or_else(|| "no reason given".into());
    ClientError::InvalidInput(format!("Parfumo refused the request: {msg}"))
}

fn refused(path: &str, body: &str) -> ClientError {
    if body.trim().is_empty() { empty_body(path) } else { ClientError::Upstream(format!("{path} refused the request: {}", excerpt(body))) }
}

/// Resolves a user-supplied list reference against the member's lists: numeric id, site
/// slug, common alias (`wish_list`, `decants`…) or display name, case-insensitively.
pub fn find_list<'a>(lists: &'a [CollectionList], reference: &str) -> Option<&'a CollectionList> {
    let wanted = reference.trim().to_lowercase();
    let key = wanted.replace([' ', '-'], "_");
    if let Ok(id) = key.parse::<u64>() {
        return lists.iter().find(|l| l.id == id);
    }
    let fixed = markers::FIXED_LISTS.iter().find(|(_, slug, aliases)| *slug == key || aliases.contains(&key.as_str())).map(|(id, _, _)| *id);
    lists
        .iter()
        .find(|l| l.slug.to_lowercase() == key || l.name.trim().to_lowercase() == wanted)
        .or_else(|| fixed.and_then(|id| lists.iter().find(|l| l.id == id)))
}

/// Site slug (or numeric id) for a reference that needs no lookup; `None` for a custom
/// list given by name.
fn direct_list_slug(reference: &str) -> Option<String> {
    let key = reference.trim().to_lowercase().replace([' ', '-'], "_");
    if !key.is_empty() && key.chars().all(|c| c.is_ascii_digit()) {
        return Some(key);
    }
    markers::FIXED_LISTS.iter().find(|(_, slug, aliases)| *slug == key || aliases.contains(&key.as_str())).map(|(_, slug, _)| slug.to_string())
}

impl ParfumoClient {
    /// Numeric perfume id (`p_id`) and `Brand/Perfume` id of a perfume reference. A purely
    /// numeric reference is taken as a `p_id` and returned without any request.
    pub async fn perfume_numeric_id(&self, reference: &str) -> Result<(u64, Option<String>), ClientError> {
        let r = reference.trim();
        if !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()) {
            return r.parse().map(|id| (id, None)).map_err(|_| ClientError::InvalidInput("perfume id out of range".into()));
        }
        let (details, ctx) = self.perfume_context(r).await?;
        Ok((ctx.p_id, Some(details.id)))
    }

    /// `Brand/Perfume` id of a perfume URL, path or id, without any request.
    pub fn perfume_ref_id(&self, reference: &str) -> Result<String, ClientError> {
        let path = self.perfume_path(reference)?;
        parse::perfume_id_from_url(&path).ok_or_else(|| ClientError::InvalidInput("not a perfume reference".into()))
    }

    /// ShelfControl page of the member's default list ("I have"): all lists with counts.
    /// Always the web page: the application interface tells whether a list is hidden, and
    /// its description, for one list per call.
    pub async fn shelf_default(&self) -> Result<Shelf, ClientError> {
        let (_, html) = self.t.get(routes::SHELFCONTROL, None, true, false).await?;
        parse::parse_shelf(&html)
    }

    /// One list of the member's own collection with every row. `reference` is a list id,
    /// slug, alias or name. A custom list given by name costs one more request.
    /// Only `list` is sure to tell `hidden` and `notes`: the application interface does not
    /// tell them for the other `lists`, which `shelf_default` gives whole.
    pub async fn shelf(&self, reference: &str) -> Result<Shelf, ClientError> {
        // Through the application interface when a session of it is held and it gives the
        // list with certainty; the ShelfControl page otherwise.
        if self.has_app_session()
            && let Some(answer) = crate::app::settle(self.app_shelf(reference).await)
        {
            return answer;
        }
        let not_found = || ClientError::NotFound(format!("collection '{}' (use parfumo_list_collections)", reference.trim()));
        let slug = match direct_list_slug(reference) {
            Some(slug) => slug,
            None => {
                let default = self.shelf_default().await?;
                let list = find_list(&default.lists, reference).ok_or_else(not_found)?;
                if list.id == default.list.id {
                    return Ok(default);
                }
                list.slug.clone()
            }
        };
        let (_, html) = self.t.get(&routes::shelfcontrol(&slug), None, true, false).await.map_err(|e| match e {
            ClientError::NotFound(_) => not_found(),
            other => other,
        })?;
        let shelf = parse::parse_shelf(&html)?;
        // An unknown list must not silently fall back to another one.
        match find_list(&shelf.lists, &slug) {
            Some(list) if list.id == shelf.list.id => Ok(shelf),
            _ => Err(not_found()),
        }
    }

    /// The site's own CSV export of a list (10 columns, UTF-8, BOM removed).
    pub async fn collection_csv(&self, list_id: u64) -> Result<String, ClientError> {
        let (_, body) = self.t.get(&routes::wardrobe_csv(list_id), None, true, false).await?;
        let body = body.trim_start_matches('\u{feff}').to_string();
        if !body.starts_with('"') {
            return Err(ClientError::ParserOutdated("collection CSV export: unexpected content".into()));
        }
        Ok(body)
    }

    /// Sends one write and checks the answer against what the route answers on success.
    /// Returns the raw body (an entry id, a JSON document, or nothing).
    pub async fn execute(&self, call: &WriteCall) -> Result<String, ClientError> {
        let form: Vec<(String, String)> = call.form.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        if call.expect == Expect::Redirect {
            let (location, body) = self.t.post_form(&call.path, &form).await?;
            return location.ok_or_else(|| refused(&call.path, &body));
        }
        let body = self.t.post_write(&call.path, &form, call.expect != Expect::Anything).await?;
        let trimmed = body.trim();
        let json = || serde_json::from_str::<serde_json::Value>(trimmed).ok();
        let ok = match call.expect {
            Expect::Anything => true,
            Expect::Body => !trimmed.is_empty(),
            Expect::One => trimmed == "1",
            Expect::StatusOk => json().is_some_and(|v| v["status"] == "ok"),
            Expect::OkTrue => json().is_some_and(|v| v["ok"] == true),
            Expect::Redirect => false,
            Expect::Success => match json() {
                Some(v) if v["success"] == true => true,
                Some(v) if v.get("success").is_some() || v.get("error").is_some() => return Err(site_refusal(&v["error"])),
                _ => false,
            },
            Expect::ResultSuccess => match json() {
                Some(v) if v["result"] == "success" => true,
                Some(v) if v.get("errors").is_some() || v.get("error").is_some() || v.get("result").is_some() => {
                    let fields = v["errors"]
                        .as_object()
                        .map(|o| o.iter().map(|(k, m)| format!("{k}: {}", m.as_str().unwrap_or_default())).collect::<Vec<_>>().join("; "))
                        .filter(|m| !m.is_empty());
                    return Err(match fields {
                        Some(m) => ClientError::InvalidInput(format!("Parfumo refused the entry: {}", excerpt(&m))),
                        None => site_refusal(&v["error"]),
                    });
                }
                _ => false,
            },
            Expect::CommentOk => match json() {
                Some(v) if v.get("val").is_some() => {
                    if v["val"].as_str().is_some_and(|m| !m.trim().is_empty()) {
                        return Err(site_refusal(&v["val"]));
                    }
                    true
                }
                _ => false,
            },
            Expect::Accepted => match json() {
                Some(v) if v.get("status").is_some() => {
                    let st = &v["status"];
                    if st == 0 || st == "0" || st == false || st.is_null() {
                        let msg = v["msg"].as_str().map(excerpt).filter(|m| !m.is_empty()).unwrap_or_else(|| "no reason given".into());
                        return Err(ClientError::InvalidInput(format!("Parfumo refused the text: {msg}")));
                    }
                    true
                }
                _ => false,
            },
        };
        if ok { Ok(body) } else { Err(refused(&call.path, &body)) }
    }

    /// Wears of one calendar month (`POST /api/assistant/calendar_month`).
    pub async fn wears_of_month(&self, year: i64, month: u32) -> Result<Vec<Wear>, ClientError> {
        let form = vec![("y".to_string(), year.to_string()), ("m".to_string(), month.to_string())];
        let body = self.t.post_fragment(routes::WEAR_CALENDAR_MONTH, &form, true, None).await?;
        if body.trim().is_empty() {
            return Err(empty_body(routes::WEAR_CALENDAR_MONTH));
        }
        parse::parse_wear_calendar(&body)
    }

    /// Wears shown on the tracker overview page, most recent first.
    pub async fn wears_overview(&self) -> Result<Vec<Wear>, ClientError> {
        let (_, html) = self.t.get(routes::WEAR_TRACKER, None, true, false).await?;
        parse::parse_wear_tracker(&html)
    }
}

// ---------------------------------------------------------------------------
// Ratings, classification, reviews and statements (jalon 3)
// ---------------------------------------------------------------------------

impl ParfumoClient {
    /// Nickname of the logged-in member, read once per session from the header of a small
    /// account page. It names the member's own pages (`/Users/{nick}/…`).
    pub async fn own_nick(&self) -> Result<String, ClientError> {
        if let Some(nick) = self.t.nick.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(nick);
        }
        let (_, html) = self.t.get(routes::LOGGED_IN_PROBE, None, true, false).await?;
        let nick = parse::parse_own_nick(&html)?;
        *self.t.nick.lock().unwrap_or_else(|e| e.into_inner()) = Some(nick.clone());
        Ok(nick)
    }

    async fn panel(&self, route: &str, form: Vec<(String, String)>) -> Result<String, ClientError> {
        let body = self.t.post_fragment(route, &form, true, None).await?;
        if body.trim().is_empty() { Err(empty_body(route)) } else { Ok(body) }
    }

    /// The member's own ratings of a perfume (0-10 per scale, `None` when not rated).
    pub async fn my_ratings_of(&self, p_id: u64) -> Result<MemberRatings, ClientError> {
        let body = self.panel(routes::RATE_PANEL, vec![("perfume_id".into(), p_id.to_string())]).await?;
        parse::parse_rate_panel(&body)
    }

    /// Classification labels of a perfume with the member's votes.
    pub async fn my_classification_of(&self, p_id: u64) -> Result<Vec<ClassChoice>, ClientError> {
        let body = self.panel(routes::CLASSIFY_PANEL, vec![("perfume_id".into(), p_id.to_string())]).await?;
        parse::parse_classify_panel(&body)
    }

    /// The member's own review or statement on a perfume; empty text when there is none.
    pub async fn my_text_on(&self, kind: TextKind, p_id: u64) -> Result<OwnDraft, ClientError> {
        let route = match kind {
            TextKind::Review => routes::REVIEW_GET_OWN,
            TextKind::Statement => routes::STATEMENT_GET_OWN,
        };
        let body = self.panel(route, vec![("p".into(), p_id.to_string()), ("lang".into(), routes::OWN_TEXT_LANG.into())]).await?;
        parse::parse_own_draft(&body, kind)
    }

    /// Delete button of the member's review or statement form for a perfume. `text_id` is
    /// the id of the review or statement when known, `0` otherwise.
    pub async fn delete_handle(&self, kind: TextKind, p_id: u64, text_id: u64) -> Result<Option<DeleteHandle>, ClientError> {
        let (route, id_field) = match kind {
            TextKind::Review => (routes::REVIEW_PANEL, "review_id"),
            TextKind::Statement => (routes::STATEMENT_PANEL, "statement_id"),
        };
        let body = self.panel(route, vec![("perfume_id".into(), p_id.to_string()), (id_field.into(), text_id.to_string())]).await?;
        parse::parse_delete_handle(&body, kind)
    }

    /// One page of the member's ratings on one scale. `order`: `d_desc` (newest), `d_asc`,
    /// `v_desc` (highest), `v_asc`.
    pub async fn my_ratings_page(&self, scale: &str, order: &str, page: u32) -> Result<Listing<RatedPerfume>, ClientError> {
        if !routes::RATING_SCALES.contains(&scale) {
            return Err(ClientError::InvalidInput("unknown rating scale".into()));
        }
        let nick = self.own_nick().await?;
        let (_, html) = self.t.get(&routes::with_page(&routes::user_ratings(&nick, scale, order), page), None, true, false).await?;
        parse::parse_my_ratings(&html, page)
    }

    /// One page of the member's own reviews or statements, newest first.
    pub async fn my_texts_page(&self, kind: TextKind, page: u32) -> Result<Listing<OwnText>, ClientError> {
        let nick = self.own_nick().await?;
        let path = match kind {
            TextKind::Review => routes::user_reviews(&nick),
            TextKind::Statement => routes::user_statements(&nick),
        };
        let (_, html) = self.t.get(&routes::with_page(&path, page), None, true, false).await?;
        match kind {
            TextKind::Review => parse::parse_user_reviews(&html, page),
            TextKind::Statement => parse::parse_user_statements(&html, page),
        }
    }
}
