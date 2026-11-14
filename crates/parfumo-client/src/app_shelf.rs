//! The member's own collection through the application interface (`docs/APP_API.md`).
//!
//! `GET /shelfcontrol` is the sheet of one list: its rows are those the ShelfControl page
//! of the web site embeds, field for field, with the lists of the member and their counts.
//! Three things the web page has are not in it:
//! - the address of a perfume. A row names its perfume by number. Its `Brand/Perfume` id is
//!   known at once when the perfume was met before; otherwise it comes from the list as the
//!   member's profile shows it (`GET /users/{nick}/collection`), the one answer about a
//!   collection that carries the slug of the brand;
//! - whether a list is hidden, and its description. `GET /wardrobe?type=` tells both, for
//!   the list asked only: the other lists of the [`Shelf`] say neither, and their `hidden`
//!   left at `false` means "not told", not "visible";
//! - the custom columns of a member who has some. An account without any answers
//!   `columns: []`; what a column looks like there was never recorded.
//!
//! So a list is read in two calls, and a third when one of its perfumes is met for the
//! first time. Whatever is not certain is left to the web site: a list the interface
//! answers another one for, an account with custom columns, a row whose address nothing
//! gives. The lists of the member with every `hidden` and description (`shelf_default`)
//! would cost one call per list here: they stay on the ShelfControl page, which has them all.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    account::find_list,
    app_db::{count, gender, number, out_of_ten, text},
    models::{CollectionItem, CollectionList, MemberRatings, Shelf},
    parse::iso_date,
    site::{markers, routes},
};

/// The sheet of one list: rows, dictionaries, lists.
const SHEET: &str = "/shelfcontrol";
/// The same list as the application shows it: here for its `list_details` and the nickname.
const WARDROBE: &str = "/wardrobe";

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// The `type` under which the interface knows a list: the slug of a fixed list, the number
/// of a custom one. It is also the slug of the list on the web site.
fn list_type(id: u64) -> String {
    markers::FIXED_LISTS.iter().find(|(fixed, _, _)| *fixed == id).map(|(_, slug, _)| slug.to_string()).unwrap_or_else(|| id.to_string())
}

/// The name a dictionary of the sheet holds under a number: a text, or a list whose first
/// element is the name. An empty dictionary is sent as `[]`.
fn named(sheet: &Value, dict: &str, nr: &Value) -> Option<String> {
    let key = match nr {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().to_string(),
        _ => return None,
    };
    let name = match sheet.get(dict)?.get(&key)? {
        Value::Array(parts) => parts.first()?,
        other => other,
    };
    name.as_str().map(str::trim).filter(|n| !n.is_empty()).map(str::to_string)
}

/// The names of the numbers a row lists under `key`.
fn names(sheet: &Value, dict: &str, row: &Value, key: &str) -> Vec<String> {
    row.get(key).and_then(Value::as_array).map(|nrs| nrs.iter().filter_map(|nr| named(sheet, dict, nr)).collect()).unwrap_or_default()
}

/// A path segment, encoded as the application encodes a nickname.
fn segment(text: &str) -> String {
    // The form encoding writes a space as `+`, and a `+` as `%2B`.
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect::<String>().replace('+', "%20")
}

/// A row of the sheet, without the address of its perfume, which the sheet does not give.
/// The fields are those of the web page, read the same way; the member's ratings are out
/// of 100 here.
fn shelf_row(sheet: &Value, row: &Value) -> Option<CollectionItem> {
    // Values of custom columns, on a sheet that has no column: not what was recorded.
    let no_value = match row.get("custom") {
        None | Some(Value::Null) => true,
        Some(Value::Object(values)) => values.is_empty(),
        Some(Value::Array(values)) => values.is_empty(),
        Some(_) => false,
    };
    if !no_value {
        return None;
    }
    let ratings = row.get("ratings").and_then(Value::as_array);
    let rating = |scale: usize| {
        let value = ratings?.get(scale)?;
        value.as_f64().or_else(|| value.as_str()?.trim().parse().ok()).filter(|v| (0.0..=100.0).contains(v)).map(out_of_ten)
    };
    let whole = |key: &str| number(row, key).filter(|n| *n >= 0.0).map(|n| n as u32);
    let positive = |key: &str| number(row, key).filter(|n| *n > 0.0);
    let day = |key: &str| positive(key).map(|ts| iso_date(ts as i64));
    Some(CollectionItem {
        entry_id: count(row, "id")?,
        perfume_id: String::new(),
        p_id: count(row, "p")?,
        name: text(row, "title")?,
        brand: row.get("brand").and_then(|nr| named(sheet, "brands", nr)),
        url: String::new(),
        year: count(row, "year").and_then(|y| u16::try_from(y).ok()),
        gender: gender(row, "gender"),
        concentration: text(row, "conc"),
        my_ratings: MemberRatings { scent: rating(0), durability: rating(1), sillage: rating(2), bottle: rating(3), pricing: rating(4) },
        community_rating: positive("community").map(|v| v as f32),
        community_votes: whole("votes"),
        quantity: whole("qty"),
        bottle_size: text(row, "bottle").or_else(|| number(row, "bottle").map(|n| n.to_string())).filter(|size| size != "0"),
        remaining_ml: positive("ml"),
        price_paid: positive("paid"),
        currency: text(row, "cur"),
        private_note: text(row, "memo"),
        added: day("added"),
        wear_count: whole("worn").unwrap_or(0),
        last_worn: day("last"),
        custom: Default::default(),
        has_statement: positive("statement").is_some(),
        has_review: positive("review").is_some(),
        perfumers: names(sheet, "perfumers", row, "perfumers"),
        accords: names(sheet, "accords", row, "accords"),
        notes: names(sheet, "notes", row, "notes"),
        seasons: names(sheet, "classes", row, "seasons"),
        occasions: names(sheet, "classes", row, "occasions"),
        country: row.get("country").and_then(|nr| named(sheet, "countries", nr)),
    })
}

/// One answer of `GET /shelfcontrol`, checked against the list that was asked.
struct Sheet {
    answer: Value,
    /// Every list of the member; `hidden` and `notes` are not told here.
    lists: Vec<CollectionList>,
    /// Position in `lists` of the list the rows belong to.
    active: usize,
}

impl ParfumoClient {
    /// A list as the sheet names it: number, `type`, name and count. `None` when its
    /// `type` is not the one its number makes certain.
    fn shelf_list(&self, list: &Value) -> Option<CollectionList> {
        let id = count(list, "id")?;
        let slug = text(list, "type").filter(|slug| *slug == list_type(id))?;
        Some(CollectionList {
            id,
            name: text(list, "name").unwrap_or_default(),
            notes: None,
            hidden: false,
            count: number(list, "count").filter(|n| *n >= 0.0)? as u32,
            custom: id > routes::LAST_FIXED_LIST_ID,
            // The page the web site serves for this slug: the interface names none.
            url: self.t.absolute(&routes::shelfcontrol(&slug)),
            slug,
        })
    }

    /// The sheet of the list of that `type`. `None` when the answer is not, with certainty,
    /// the sheet that was asked, or holds what was never recorded.
    async fn app_sheet(&self, list: &str) -> Result<Option<Sheet>, ClientError> {
        let answer = self.app_get(SHEET, &[pair("list", list)]).await?;
        if text(&answer, "status").as_deref() != Some("success") || !answer.get("rows").is_some_and(Value::is_array) {
            return Ok(None);
        }
        // Custom columns: their shape was never recorded, and a write goes by their number.
        if !answer.get("columns").and_then(Value::as_array).is_some_and(Vec::is_empty) {
            return Ok(None);
        }
        let lists = answer.get("lists").and_then(Value::as_array).and_then(|all| all.iter().map(|l| self.shelf_list(l)).collect::<Option<Vec<_>>>());
        let Some(lists) = lists else { return Ok(None) };
        // An unknown list must not silently be another one: the answer says whose rows these are.
        let active = count(&answer, "list").and_then(|nr| lists.iter().position(|l| l.id == nr)).filter(|at| lists[*at].slug == list);
        Ok(active.map(|active| Sheet { answer, lists, active }))
    }

    /// The `Brand/Perfume` id of the perfumes met before, among those wanted.
    fn known_perfumes(&self, wanted: &HashSet<u64>) -> HashMap<u64, String> {
        let met = self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner());
        met.values().filter(|(p_id, _)| wanted.contains(p_id)).map(|(p_id, id)| (*p_id, id.clone())).collect()
    }

    /// The `Brand/Perfume` id of the perfumes of a list, as the profile of the member shows
    /// it (`GET /users/{nick}/collection`). They are remembered: the next read needs no call.
    async fn app_shown_perfumes(&self, nick: &str, list: &str) -> Result<HashMap<u64, String>, ClientError> {
        let shown = self.app_get(&format!("/users/{}/collection", segment(nick)), &[pair("type", list)]).await?;
        // The answer repeats the list it shows.
        if text(&shown, "type").as_deref() != Some(list) {
            return Ok(HashMap::new());
        }
        let perfumes = shown.get("perfumes").and_then(Value::as_array);
        let ids: HashMap<u64, String> = perfumes
            .into_iter()
            .flatten()
            .filter_map(|p| Some((count(p, "id")?, format!("{}/{}", text(p, "b_name_url")?, text(p, "p_name_url")?))))
            .collect();
        let mut met = self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner());
        for (p_id, id) in &ids {
            met.insert(id.to_lowercase(), (*p_id, id.clone()));
        }
        Ok(ids)
    }

    /// One list of the member's own collection with every row, when the interface gives it
    /// with certainty. `None` otherwise: the caller then reads the ShelfControl page.
    pub(crate) async fn app_shelf(&self, reference: &str) -> Result<Option<Shelf>, ClientError> {
        match self.app_shelf_of(reference).await {
            // A list, or a call, the interface does not know: the web site says what it is.
            Err(ClientError::NotFound(_) | ClientError::InvalidInput(_)) => Ok(None),
            other => other,
        }
    }

    async fn app_shelf_of(&self, reference: &str) -> Result<Option<Shelf>, ClientError> {
        // The list asked, as `shelf` reads a reference: a number, a slug or an alias needs
        // no lookup; a name is looked up among the lists the default sheet comes with.
        let key = reference.trim().to_lowercase().replace([' ', '-'], "_");
        let direct = if !key.is_empty() && key.chars().all(|c| c.is_ascii_digit()) {
            match key.parse::<u64>() {
                Ok(id) if id > 0 => Some(list_type(id)),
                _ => return Ok(None),
            }
        } else {
            markers::FIXED_LISTS
                .iter()
                .find(|(_, slug, aliases)| *slug == key || aliases.contains(&key.as_str()))
                .map(|(_, slug, _)| slug.to_string())
        };
        let sheet = match direct {
            Some(list) => self.app_sheet(&list).await?,
            None => match self.app_sheet(&list_type(routes::LIST_HAVE)).await? {
                Some(default) => match find_list(&default.lists, reference).map(|list| (list.id, list.slug.clone())) {
                    Some((id, _)) if id == default.lists[default.active].id => Some(default),
                    Some((_, slug)) => self.app_sheet(&slug).await?,
                    None => None,
                },
                None => None,
            },
        };
        let Some(Sheet { answer, mut lists, active }) = sheet else { return Ok(None) };
        let list = lists[active].slug.clone();
        // A row that lacks what names it is not left out: the web page has them all.
        let rows = answer.get("rows").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let Some(mut items) = rows.iter().map(|row| shelf_row(&answer, row)).collect::<Option<Vec<_>>>() else { return Ok(None) };

        // Whether the list is hidden, and its description: only this call tells.
        let shown = self.app_get(WARDROBE, &[pair("type", &list)]).await?;
        let Some(details) = shown.get("list_details").filter(|details| count(details, "id") == Some(lists[active].id)) else { return Ok(None) };
        lists[active].hidden = match text(details, "hidden").map(|hidden| hidden.to_lowercase()).as_deref() {
            Some("yes") => true,
            Some("no") => false,
            _ => return Ok(None),
        };
        lists[active].notes = text(details, "notes");

        // The address of every perfume: remembered, or read once from the profile of the member.
        let wanted: HashSet<u64> = items.iter().map(|item| item.p_id).collect();
        let mut ids = self.known_perfumes(&wanted);
        if ids.len() < wanted.len() {
            let Some(nick) = text(&shown, "nick_name") else { return Ok(None) };
            ids.extend(self.app_shown_perfumes(&nick, &list).await?);
        }
        for item in &mut items {
            // A perfume the profile does not show, as a hidden list may be: no address is made up.
            let Some(id) = ids.get(&item.p_id) else { return Ok(None) };
            item.url = self.t.absolute(&routes::perfume(id));
            item.perfume_id = id.clone();
        }
        Ok(Some(Shelf { list: lists[active].clone(), lists, columns: Vec::new(), items }))
    }
}
