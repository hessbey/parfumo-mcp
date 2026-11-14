//! Member profiles and their collection lists through the application interface
//! (`docs/APP_API.md`): `GET /users/{nick}` and `GET /users/{nick}/collection`.
//!
//! The interface tells less of a member than the profile page: no rank, no activity, no
//! achievements, no signature scent, and of the tabs only the reviews, the statements and
//! the photos. What it has no value for stays empty, and the reads that need those fields
//! keep the web page (see `member_profile`).
//!
//! Only what was recorded is read here. Custom lists never were (`lists` came back empty;
//! the application reads `{id, name, count, icon}` of each): a member who has some is left
//! to the web site, and so is a custom list. The reviews and statements a member wrote are
//! not read at all: `/users/{nick}/reviews` and `/statements` were only seen empty, so the
//! shape of their rows is not known.

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    app_db::{count, gender, number, text},
    models::{EntityRef, MemberCollectionItem, MemberList, MemberProfile, ProfileCounter, WardrobeCall},
    parse,
    site::routes,
};

/// The fixed lists, in the order of the profile page: their type in the interface, the
/// last segment of their address on the web site, and the name the web site shows.
const FIXED_LISTS: [(&str, &str, &str); 7] = [
    ("have", "I_have", "I have"),
    ("had", "I_had", "I had"),
    ("want", "Wish_List", "Wish List"),
    ("memo", "Watch_List", "Watch List"),
    ("tested", "Tested", "Tested"),
    ("filling", "Abfuellungen", "Decants"),
    ("mini", "Miniaturen", "Miniatures"),
];

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// A nickname as a segment of a path, encoded as the application does.
fn segment(nick: &str) -> String {
    url::form_urlencoded::byte_serialize(nick.as_bytes()).collect()
}

/// A whole number that may be zero.
fn whole(v: &Value, key: &str) -> Option<u32> {
    number(v, key).filter(|n| *n >= 0.0 && n.fract() == 0.0 && *n <= f64::from(u32::MAX)).map(|n| n as u32)
}

/// The number of perfumes in each fixed list, in the order of `FIXED_LISTS`. `None` when
/// the answer does not count them all.
fn fixed_counts(user: &Value) -> Option<[u32; 7]> {
    let counted = user.get("collection")?;
    let mut counts = [0; 7];
    for (slot, (kind, _, _)) in counts.iter_mut().zip(FIXED_LISTS) {
        *slot = whole(counted, kind)?;
    }
    Some(counts)
}

/// A perfume as the answers about a member carry it: the one worn, or a row of a list.
struct MemberPerfume {
    p_id: u64,
    /// `Brand/Perfume`.
    id: String,
    name: String,
    brand: Option<String>,
    url: String,
    image_url: Option<String>,
}

impl ParfumoClient {
    /// `{id, p_name, p_name_url, concentration, b_name, b_name_url, url_image_240, …}`.
    /// `None` when the number or a slug is missing: an address is never guessed.
    fn app_member_perfume(&self, row: &Value) -> Option<MemberPerfume> {
        let p_id = count(row, "id")?;
        let id = format!("{}/{}", text(row, "b_name_url")?, text(row, "p_name_url")?);
        let name = text(row, "p_name")?;
        // The concentration is part of the name on the web site, and a field of its own
        // here: `Sauvage` and `Eau de Parfum`, as recorded on the rows of a note. The
        // perfumes recorded with a member have none.
        let name = match text(row, "concentration") {
            Some(concentration) if !name.ends_with(&concentration) => format!("{name} {concentration}"),
            _ => name,
        };
        self.remember_perfume(&id, p_id);
        Some(MemberPerfume {
            p_id,
            name,
            brand: text(row, "b_name"),
            url: self.t.absolute(&routes::perfume(&id)),
            image_url: text(row, "url_image_240"),
            id,
        })
    }

    /// What `GET /users/{nick}` tells of the member asked for. `None` for a nickname the
    /// interface does not know (the application expects a 404, which was not recorded) and
    /// for an answer about someone else: the web site then says what there is.
    async fn app_member(&self, nick: &str) -> Result<Option<Value>, ClientError> {
        let answer = match self.app_get(&format!("/users/{}", segment(nick)), &[]).await {
            Err(ClientError::NotFound(_)) => return Ok(None),
            answer => answer?,
        };
        let asked = nick.to_lowercase();
        Ok(answer.get("user").filter(|user| text(user, "nick_name").is_some_and(|n| n.to_lowercase() == asked)).cloned())
    }

    /// The public profile of a member, in one call. `None` when the interface cannot tell
    /// it whole: an unknown nickname, custom lists, an answer of another shape.
    pub(crate) async fn app_member_profile(&self, nick: &str) -> Result<Option<MemberProfile>, ClientError> {
        let Some(user) = self.app_member(nick).await? else { return Ok(None) };
        // Custom lists were never recorded: how the web site addresses them is not known here.
        if !user.get("lists").and_then(Value::as_array).is_some_and(Vec::is_empty) {
            return Ok(None);
        }
        let (Some(nick), Some(counts)) = (text(&user, "nick_name"), fixed_counts(&user)) else { return Ok(None) };
        // The tabs of the profile page the interface counts; like the page, it shows none for a zero.
        let tab = |label: &str, key: &str, path: Option<String>| {
            let count = user.get("stats").and_then(|stats| whole(stats, key)).filter(|n| *n > 0)?;
            Some(ProfileCounter { label: label.to_string(), count, url: path.map(|p| self.t.absolute(&p)) })
        };
        let counters = [
            tab("Reviews", "reviews", Some(routes::user_reviews(&nick))),
            tab("Statements", "statements", Some(routes::user_statements(&nick))),
            // Which page of the web site these photos are on is not told.
            tab("Photos", "photos", None),
        ];
        let wearing = match user.get("wearing") {
            None | Some(Value::Null) => None,
            // A perfume is worn and cannot be read: the profile must not say that none is.
            Some(worn) => match self.app_member_perfume(worn) {
                Some(perfume) => Some(perfume),
                None => return Ok(None),
            },
        };
        let collections = FIXED_LISTS
            .iter()
            .zip(counts)
            .filter(|(_, count)| *count > 0)
            .map(|((_, list, name), count)| MemberList {
                id: list.to_string(),
                name: name.to_string(),
                count: Some(count),
                url: self.t.absolute(&routes::user_collection(&nick, list)),
            })
            .collect();
        Ok(Some(MemberProfile {
            url: self.t.absolute(&routes::user(&nick)),
            member_id: count(&user, "id"),
            gender: gender(&user, "gender"),
            country: text(&user, "country"),
            // The moment of the registration, where the page shows a date: its day in UTC,
            // written so that it is not taken for the date of the page.
            member_since: count(&user, "reg_date").and_then(|at| i64::try_from(at).ok()).map(parse::iso_date),
            points: whole(&user, "scores"),
            followers: whole(&user, "followers"),
            counters: counters.into_iter().flatten().collect(),
            wearing: wearing.map(|p| EntityRef { id: p.id, numeric_id: Some(p.p_id), name: p.name, url: p.url }),
            collections,
            nick,
            // Not in the answer: the rank, the activity, the achievements, the signature scent.
            ..MemberProfile::default()
        }))
    }

    /// One fixed list of a member, whole, in two calls: the profile, for the number of the
    /// member and the size of the list, then the list. `list` is the last segment of the
    /// address of the list on the web site. `None` for a custom list, and whenever the
    /// answer cannot be told to be the whole list that was asked.
    pub(crate) async fn app_member_collection(
        &self,
        nick: &str,
        list: &str,
    ) -> Result<Option<(WardrobeCall, Vec<MemberCollectionItem>)>, ClientError> {
        let Some(slot) = FIXED_LISTS.iter().position(|(_, known, _)| known.eq_ignore_ascii_case(list)) else { return Ok(None) };
        let (kind, type_url, _) = FIXED_LISTS[slot];
        let Some(user) = self.app_member(nick).await? else { return Ok(None) };
        let (Some(member_id), Some(nick), Some(counts)) = (count(&user, "id"), text(&user, "nick_name"), fixed_counts(&user)) else {
            return Ok(None);
        };
        let answer = match self.app_get(&format!("/users/{}/collection", segment(&nick)), &[pair("type", kind)]).await {
            Err(ClientError::NotFound(_)) => return Ok(None),
            answer => answer?,
        };
        // The answer names its list: another one is not what was asked.
        if text(&answer, "type").as_deref() != Some(kind) {
            return Ok(None);
        }
        let rows = answer
            .get("perfumes")
            .and_then(Value::as_array)
            .ok_or_else(|| ClientError::ParserOutdated("/app/v1/users/collection: no perfumes".into()))?;
        let items: Vec<MemberCollectionItem> = rows
            .iter()
            .filter_map(|row| self.app_member_perfume(row))
            .map(|p| MemberCollectionItem {
                p_id: p.p_id,
                // The interface does not number the entries of another member's list: zero is none.
                entry_id: 0,
                name: p.name,
                perfume_id: Some(p.id),
                brand: p.brand,
                url: Some(p.url),
                image_url: p.image_url,
            })
            .collect();
        // The call takes no page, and only a list of two perfumes was recorded: whether a
        // long one comes whole is not known. The profile counts the list, so a list that
        // is not of that size, or has a row that could not be read, is left to the web site.
        if items.len() != counts[slot] as usize {
            return Ok(None);
        }
        let call = WardrobeCall { member_id, nick, list_type: kind.to_string(), type_url: type_url.to_string(), total: counts[slot].to_string() };
        Ok(Some((call, items)))
    }
}
