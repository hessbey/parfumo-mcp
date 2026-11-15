//! Database reads through the application interface (`docs/APP_API.md`): the same models
//! as the HTML parsers produce, filled from JSON.
//!
//! The interface answers a read only when its answer is as complete as the one of the web
//! site. Whatever it cannot tell (a reference its own lookup does not find, a listing without
//! its count, a refusal in JSON) is left to the web site, which has the last word. Reviews
//! and statements are never read here: the interface gives one rating where the web page
//! gives five, a relative date and no total.

use std::{collections::HashMap, hash::Hash};

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    models::{ClassGroup, ClassShare, EntityRef, Gender, Listing, NotePyramid, PerfumeDetails, PerfumeSummary, RatingScore, SearchQuery},
    site::routes,
};

/// Rows asked per page of a listing: what the application asks.
const PER_PAGE: u32 = 20;
/// The order of a search when none is asked: the most rated first, which is the order of the
/// web search (measured: the same perfumes in the same order, for words and for a brand).
/// Without an order the interface lists by name.
const DEFAULT_ORDER: &str = "nr_desc";
/// Perfumes whose number is remembered at most. A server runs for weeks and every page of
/// a listing brings twenty of them: past this the memory is emptied and filled again.
const MAX_PERFUMES: usize = 5_000;

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// A non-empty text field.
pub(crate) fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// A number, which the interface sends as a number or as text.
pub(crate) fn number(v: &Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// A positive whole number.
pub(crate) fn count(v: &Value, key: &str) -> Option<u64> {
    number(v, key).filter(|n| *n >= 1.0).map(|n| n as u64)
}

/// The count of a listing (`matches`), zero included. `None` when the answer carries none:
/// that is not a count of zero, and a listing without its count is not as complete as the
/// web page, which always tells how many there are.
pub(crate) fn matches(listing: &Value) -> Option<u64> {
    number(listing, "matches").filter(|n| *n >= 0.0).map(|n| n as u64)
}

/// The interface names genders in German, whatever the language.
pub(crate) fn gender(v: &Value, key: &str) -> Option<String> {
    match text(v, key)?.as_str() {
        "maennlich" | "m" => Some("men".into()),
        "weiblich" | "f" | "w" => Some("women".into()),
        "unisex" | "u" => Some("unisex".into()),
        _ => None,
    }
}

/// A rating out of 100 as the site shows it, out of 10 with one decimal.
pub(crate) fn out_of_ten(percent: f64) -> f32 {
    ((percent / 10.0) * 10.0).round() as f32 / 10.0
}

/// A share in percent, with one decimal.
fn tenth(percent: f64) -> f32 {
    (percent * 10.0).round() as f32 / 10.0
}

/// A text of the site as plain text: line breaks kept, markup and entities removed.
pub(crate) fn plain(html: &str) -> String {
    // The site writes a line break as `<br />` followed by the raw one it stands for: the
    // two are one break. A raw break alone is a break too: what a text without `<br />`
    // looks like was not recorded, and its lines must not be glued together.
    let mut broken = html.replace("\r\n", "\n").replace('\r', "\n");
    for br in ["<br />", "<br/>", "<br>"] {
        broken = broken.replace(&format!("{br}\n"), "\n").replace(br, "\n");
    }
    let text: String = scraper::Html::parse_fragment(&broken).root_element().text().collect();
    let mut lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines.dedup_by(|a, b| a.is_empty() && b.is_empty());
    lines.join("\n").trim().to_string()
}

/// A text field of a record, cleaned as every text of the site is; `None` when nothing is left.
fn plain_text(v: &Value, key: &str) -> Option<String> {
    text(v, key).map(|t| plain(&t)).filter(|t| !t.is_empty())
}

/// Why the interface has no answer to a read.
#[derive(Debug, PartialEq, Eq)]
enum Unserved {
    /// It has nothing there: a 404, a refusal that says "not found", an answer about nothing.
    Absent,
    /// Any other refusal worded in JSON, with its words when it has some: it says nothing
    /// of whether what was asked exists.
    Refused(String),
}

/// As many characters of a refusal as are passed on.
const MAX_REFUSAL_CHARS: usize = 200;

/// The refusal an answer is, when it is one: the interface words some of them in JSON under
/// status 200 (`{"error": "Brand not found"}`, `{"status": "error", …}`).
fn refusal(answer: &Value) -> Option<Unserved> {
    let error = answer.get("error").is_some_and(|e| !matches!(e, Value::Null | Value::Bool(false)) && e.as_str() != Some(""));
    if !error && text(answer, "status").as_deref() != Some("error") {
        return None;
    }
    let words = ["message", "error"].iter().find_map(|key| text(answer, key)).unwrap_or_default();
    // On one line and of a bounded length: these words end in an error message.
    let words: String = words.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_REFUSAL_CHARS).collect();
    Some(if words.to_lowercase().contains("not found") { Unserved::Absent } else { Unserved::Refused(words) })
}

/// Whether a part of an answer holds nothing: absent in all but name.
fn holds_nothing(part: &Value) -> bool {
    match part {
        Value::Null | Value::Bool(false) => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(entries) => entries.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        _ => false,
    }
}

/// What a read by number reports when the interface gives no record and no web page stands
/// behind the number: not found only when the interface has nothing under it. Any other
/// refusal says nothing of the perfume, and is reported as a refusal, with its words.
fn no_record(p_id: u64, why: Unserved) -> ClientError {
    let path = format!("{}/perfume_details/{p_id}", crate::app::PREFIX);
    match why {
        Unserved::Absent => ClientError::NotFound(format!("/perfume_details/{p_id}")),
        Unserved::Refused(words) if words.is_empty() => ClientError::Upstream(format!("{path} was refused")),
        Unserved::Refused(words) => ClientError::Upstream(format!("{path} was refused: {words}")),
    }
}

/// Adds to a memory of identifiers that must stay bounded: once full it is emptied, and
/// what is needed again is looked up again.
pub(crate) fn remember<K: Eq + Hash, V>(memory: &mut HashMap<K, V>, cap: usize, key: K, value: V) {
    if memory.len() >= cap && !memory.contains_key(&key) {
        memory.clear();
    }
    memory.insert(key, value);
}

/// The names of a list of notes (`[{id, name, …}]`), without the empty placeholders.
fn note_names(p: &Value, key: &str) -> Vec<String> {
    p.get(key).and_then(Value::as_array).map(|notes| notes.iter().filter_map(|n| text(n, "name")).collect()).unwrap_or_default()
}

/// The words of a slug, as the search of the application takes them.
fn words(slug: &str) -> String {
    slug.replace(['_', '-'], " ")
}

/// Among the brands a search found, the number of the one with that slug, without regard to case.
pub(crate) fn brand_by_slug(brands: &[Value], slug: &str) -> Option<u64> {
    brands.iter().find(|b| text(b, "b_name_url").is_some_and(|u| u.eq_ignore_ascii_case(slug))).and_then(|b| count(b, "b_id"))
}

/// The entries of a list with their place in it. The interface also sends lists as objects
/// keyed by position: their entries are taken by the number of their key, not by the order
/// of the keys as text (`"10"` sorts before `"2"`).
fn by_position(list: &Value) -> Vec<(usize, &Value)> {
    match list {
        Value::Array(entries) => entries.iter().enumerate().collect(),
        Value::Object(entries) => {
            let mut keyed: Vec<(usize, &Value)> = entries.iter().filter_map(|(key, entry)| Some((key.parse().ok()?, entry))).collect();
            keyed.sort_by_key(|(place, _)| *place);
            keyed
        }
        _ => Vec::new(),
    }
}

/// The entry at a place of a list read by `by_position`.
fn at<'a>(list: &[(usize, &'a Value)], place: usize) -> Option<&'a Value> {
    list.iter().find(|(p, _)| *p == place).map(|(_, entry)| *entry)
}

/// The place of the "Audience & Style" chart in `c`, the only chart that is not a pie.
const AUDIENCE: usize = 3;
/// The charts of the community classification: their place in `c` and `c_total`, and their
/// name. Listed in the order of the web record, which is not the one of the interface.
const CLASS_GROUPS: [(usize, &str); 4] = [(0, "Type"), (2, "Occasion"), (1, "Season"), (AUDIENCE, "Audience & Style")];
/// The two pairs of the "Audience & Style" chart by the `ct` of their poles, in the order
/// of the web chart, with the name of a pole for when the answer does not name it.
const POLE_PAIRS: [[(u64, &str); 2]; 2] = [[(2, "Masculine"), (1, "Feminine")], [(3, "Modern"), (4, "Classic")]];
/// As many accords as the web page shows.
const MAX_ACCORDS: usize = 5;
/// The chart the accords are taken from.
const ACCORDS_GROUP: &str = "Type";
/// The five rating scales: name, then the suffix of their fields.
const SCALES: [(&str, &str); 5] = [("scent", ""), ("durability", "_dur"), ("sillage", "_sillage"), ("bottle", "_bottle"), ("pricing", "_pricing")];

/// What the listed poles of "Audience & Style" may leave of 100 by rounding alone.
const ROUNDING: f64 = 0.05;

/// The shares of a pie chart (Type, Season, Occasion), strongest first, as the web chart
/// gives them: out of the slices listed. The `p` of the interface also counts the slices
/// it does not list, so those of a "Type" chart do not add up to 100. `None` when a slice
/// has no name or no share that can be read: the others would be scaled up to fill its
/// place, and nothing would tell.
fn pie_shares(chart: &Value) -> Option<Vec<ClassShare>> {
    let mut slices = Vec::new();
    for (_, slice) in by_position(chart) {
        let (label, share) = (text(slice, "n")?, number(slice, "p").filter(|p| p.is_finite() && *p >= 0.0)?);
        // A slice nobody voted for is not on the web chart.
        if share > 0.0 {
            slices.push((label, share));
        }
    }
    let listed: f64 = slices.iter().map(|(_, p)| p).sum();
    slices.sort_by(|a, b| b.1.total_cmp(&a.1));
    Some(slices.into_iter().map(|(label, p)| ClassShare { label, percent: tenth(p / listed * 100.0) }).collect())
}

/// The shares of the "Audience & Style" chart as the web chart gives them: two pairs that
/// each add up to 100 (Masculine and Feminine, Modern and Classic). The interface gives one
/// share over the four poles and leaves the weakest out: its share is what the three
/// others leave of 100. When the listed poles leave nothing of 100, every pole left out
/// holds nothing. A pair that cannot be rebuilt (what is left of 100 belongs to several
/// poles left out) is left out and named in `warnings`; a pair nobody chose between is
/// left out without a word, there is no chart of it.
fn pole_shares(chart: &Value, poles: Option<&Value>, warnings: &mut Vec<String>) -> Vec<ClassShare> {
    let entries = by_position(chart);
    // Nobody voted: there is no chart, and nothing to tell.
    if entries.is_empty() {
        return Vec::new();
    }
    let slices: Vec<(u64, f64)> =
        entries.iter().filter_map(|(_, s)| Some((count(s, "ct")?, number(s, "p").filter(|p| p.is_finite() && *p >= 0.0)?))).collect();
    let listed = |ct: u64| slices.iter().find(|(pole, _)| *pole == ct).map(|(_, p)| *p);
    let known = || POLE_PAIRS.iter().flatten().map(|(ct, _)| *ct);
    // An entry without its share, or the share of a pole this code does not know, would
    // be taken for the missing pole's.
    let trusted = slices.len() == entries.len() && slices.iter().all(|(ct, _)| known().any(|pole| pole == *ct));
    let missing: Vec<u64> = known().filter(|ct| listed(*ct).is_none()).collect();
    let left = 100.0 - slices.iter().map(|(_, p)| p).sum::<f64>();
    // The share of a pole left out: all that is left when it is the only one, nothing when
    // nothing is left. Anything else left cannot be told apart between the poles left out.
    let restored = match missing.len() {
        1 => Some(left.max(0.0)),
        _ if left <= ROUNDING => Some(0.0),
        _ => None,
    };
    let share = |ct: u64| listed(ct).or(restored.filter(|_| missing.contains(&ct))).filter(|_| trusted);
    let name = |ct: u64, fallback: &str| {
        let named = |list: &Value| by_position(list).into_iter().find(|(_, e)| count(e, "ct") == Some(ct)).and_then(|(_, e)| text(e, "n"));
        poles.and_then(named).or_else(|| named(chart)).unwrap_or_else(|| fallback.to_string())
    };
    let mut shares = Vec::new();
    for [(first, first_name), (second, second_name)] in POLE_PAIRS {
        match (share(first), share(second)) {
            (Some(a), Some(b)) if a + b > 0.0 => {
                let percent = tenth(a / (a + b) * 100.0);
                shares.push(ClassShare { label: name(first, first_name), percent });
                shares.push(ClassShare { label: name(second, second_name), percent: tenth(100.0 - f64::from(percent)) });
            }
            // Nobody chose between the two: there is no pair to show, and nothing is lost.
            (Some(_), Some(_)) => {}
            _ => warnings.push(format!(
                "classification: the {first_name}/{second_name} shares of \"Audience & Style\" cannot be worked out from what the application interface gives"
            )),
        }
    }
    shares
}

impl ParfumoClient {
    /// Remembers the number of a perfume under its `Brand/Perfume` id. One id per number:
    /// a perfume renamed on the site is met again under its new slug, and the old one must
    /// not be handed out any more.
    fn remember_perfume(&self, id: &str, p_id: u64) {
        let mut known = self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner());
        known.retain(|_, (number, _)| *number != p_id);
        remember(&mut known, MAX_PERFUMES, id.to_lowercase(), (p_id, id.to_string()));
    }

    /// The address an answer gives today for a perfume already remembered: a number known
    /// under another address takes this one. A number not met before is not added: these
    /// rows come with the lookup of a brand, and what a lookup happens to list must not
    /// change which calls a later read makes.
    fn refresh_perfume(&self, id: &str, p_id: u64) {
        let mut known = self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner());
        let dated = known.iter().find(|(_, (number, address))| *number == p_id && address != id).map(|(key, _)| key.clone());
        if let Some(key) = dated {
            known.remove(&key);
            known.insert(id.to_lowercase(), (p_id, id.to_string()));
        }
    }

    /// Refreshes the addresses of the perfumes an answer of `GET /search` lists
    /// (`{p_id, b_name_url, p_name_url}` each): a perfume renamed on the site must not keep
    /// its old address until one of its own listings is read again.
    fn refresh_perfumes(&self, found: &Value) {
        for row in found.get("perfumes").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(p_id), Some(brand_url), Some(name_url)) = (count(row, "p_id"), text(row, "b_name_url"), text(row, "p_name_url")) {
                self.refresh_perfume(&format!("{brand_url}/{name_url}"), p_id);
            }
        }
    }

    /// A read of the interface, or why it has no answer: an address it answers 404 to, or
    /// a refusal worded in JSON under status 200.
    async fn app_answer(&self, path: &str, params: &[(String, String)]) -> Result<Result<Value, Unserved>, ClientError> {
        match self.app_get(path, params).await {
            Ok(answer) => Ok(match refusal(&answer) {
                // The words of a refusal end in an error message, where the session token
                // must never be: words that repeat it are not passed on.
                Some(Unserved::Refused(words)) if self.t.app_token().is_some_and(|token| !token.is_empty() && words.contains(&token)) => {
                    Err(Unserved::Refused(String::new()))
                }
                Some(why) => Err(why),
                None => Ok(answer),
            }),
            Err(ClientError::NotFound(_)) => Ok(Err(Unserved::Absent)),
            Err(e) => Err(e),
        }
    }

    /// A read of the interface, or `None` when it has nothing to say: an address it answers
    /// 404 to, or a refusal in JSON. The caller then leaves the read to the web site.
    pub(crate) async fn app_read(&self, path: &str, params: &[(String, String)]) -> Result<Option<Value>, ClientError> {
        Ok(self.app_answer(path, params).await?.ok())
    }

    /// The brands the search of the application finds for a slug or a name (`GET /search`):
    /// `{b_id, b_name, b_name_url}` each. None when the search has nothing to say.
    pub(crate) async fn app_brands_like(&self, reference: &str) -> Result<Vec<Value>, ClientError> {
        let Some(found) = self.app_read("/search", &[pair("filter", words(reference))]).await? else { return Ok(Vec::new()) };
        self.refresh_perfumes(&found);
        Ok(found.get("brands").and_then(Value::as_array).cloned().unwrap_or_default())
    }

    /// Among the rows of a listing, the number of the perfume that bears an id, without
    /// regard to case. Reading the rows records their numbers, but the number looked for
    /// is taken from its row: the memory is emptied when it is full, which may happen
    /// between the row and the end of the listing.
    fn perfume_among(&self, rows: &[Value], id: &str) -> Option<u64> {
        let wanted = id.to_lowercase();
        let mut found = None;
        for row in rows {
            self.summary_from_index_row(row);
            if let (Some(brand_url), Some(name_url)) = (text(row, "b_name_url"), text(row, "p_name_url"))
                && format!("{brand_url}/{name_url}").to_lowercase() == wanted
            {
                found = found.or(count(row, "p_id"));
            }
        }
        found
    }

    /// The number of a perfume from a reference: a number, a Parfumo URL or a
    /// `Brand/Perfume` id. An id met in a listing is known at once; another is looked up
    /// through its brand, in two calls. `None` when that lookup does not find it, which is
    /// not an answer of Parfumo: the lookup reads one page of what the words of the slug
    /// match, and the web site knows every address.
    pub(crate) async fn app_perfume_number(&self, reference: &str) -> Result<Option<u64>, ClientError> {
        let reference = reference.trim();
        if !reference.is_empty() && reference.chars().all(|c| c.is_ascii_digit()) {
            return reference.parse().map(Some).map_err(|_| ClientError::InvalidInput("not a perfume number".into()));
        }
        let id = self.perfume_ref_id(reference)?;
        let known = |c: &Self| c.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner()).get(&id.to_lowercase()).map(|(p_id, _)| *p_id);
        if let Some(p_id) = known(self) {
            return Ok(Some(p_id));
        }
        let (brand_url, name_url) = id.split_once('/').ok_or_else(|| ClientError::InvalidInput("not a perfume reference".into()))?;
        let Some(brand) = brand_by_slug(&self.app_brands_like(brand_url).await?, brand_url) else { return Ok(None) };
        let params = [
            pair("page", 1),
            pair("per_page", PER_PAGE),
            pair("filter", words(name_url)),
            pair("b_id", brand),
            pair("g_f", 1),
            pair("g_m", 1),
            pair("g_u", 1),
        ];
        let Some(listing) = self.app_read("/perfumes", &params).await? else { return Ok(None) };
        Ok(self.perfume_among(listing.get("rows").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default(), &id))
    }

    /// The number of a perfume from a reference, for a read that decides by itself what a
    /// reference the lookup does not find becomes: here it is reported as not found.
    pub(crate) async fn app_perfume_id(&self, reference: &str) -> Result<u64, ClientError> {
        match self.app_perfume_number(reference).await? {
            Some(p_id) => Ok(p_id),
            None => Err(ClientError::NotFound(self.perfume_ref_id(reference)?)),
        }
    }

    /// The full record of a perfume the interface finds (`GET /perfume_details/{number}`).
    /// A perfume it has nothing about is not found.
    pub(crate) async fn app_get_perfume(&self, reference: &str) -> Result<PerfumeDetails, ClientError> {
        let p_id = self.app_perfume_id(reference).await?;
        self.app_perfume_by_number(p_id).await?.map_err(|why| no_record(p_id, why))
    }

    /// The full record of a perfume, for the read that has the web page behind it. `None`
    /// when the interface does not find the reference, has nothing under the number it
    /// remembered for it (a perfume merged or removed since), or refuses to answer: the
    /// web page then answers. A bare number has no address on the web site: the interface
    /// alone tells whether it exists, and a refusal about something else is reported as one.
    pub(crate) async fn app_perfume_record(&self, reference: &str) -> Result<Option<PerfumeDetails>, ClientError> {
        let Some(p_id) = self.app_perfume_number(reference).await? else { return Ok(None) };
        match self.app_perfume_by_number(p_id).await? {
            Ok(record) => Ok(Some(record)),
            Err(why) if reference.trim().chars().all(|c| c.is_ascii_digit()) => Err(no_record(p_id, why)),
            Err(Unserved::Absent) => {
                self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, (number, _)| *number != p_id);
                Ok(None)
            }
            // Nothing says that the number is not the one of the perfume any more: it is kept.
            Err(Unserved::Refused(_)) => Ok(None),
        }
    }

    /// The record of the perfume that bears a number, or why the interface gives none: it
    /// has nothing under the number, or refuses. Filled as the web record is: same scales,
    /// same order, same addresses. Two parts of the web record are not in the answer and
    /// stay absent: the rank label ("Ranked 31 in Men's Perfume") and, for the facts,
    /// anything but their English text.
    async fn app_perfume_by_number(&self, p_id: u64) -> Result<Result<PerfumeDetails, Unserved>, ClientError> {
        let record = match self.app_answer(&format!("/perfume_details/{p_id}"), &[]).await? {
            Ok(record) => record,
            Err(why) => return Ok(Err(why)),
        };
        let outdated = |what: &str| ClientError::ParserOutdated(format!("/app/v1/perfume_details: {what}"));
        // An answer under status 200 that holds no perfume, as the interface gives for a
        // brand or a perfumer it does not have, or that holds another perfume than the one
        // asked: nothing is under this number. It is not a change of the interface.
        let Some(p) = record.get("p").filter(|p| !holds_nothing(p) && count(p, "id").is_none_or(|id| id == p_id)) else {
            return Ok(Err(Unserved::Absent));
        };
        // A perfume without its brand is another matter: every perfume has one.
        let b = record.get("b").filter(|b| !holds_nothing(b)).ok_or_else(|| outdated("no brand"))?;
        let (brand_url, name_url) =
            (text(b, "name_url").ok_or_else(|| outdated("no brand id"))?, text(p, "name_url").ok_or_else(|| outdated("no perfume id"))?);
        let id = format!("{brand_url}/{name_url}");
        self.remember_perfume(&id, p_id);

        let entity = |prefix: &str, slug: String, numeric: Option<u64>, name: String| EntityRef {
            url: self.t.absolute(&format!("{prefix}{slug}")),
            id: slug,
            numeric_id: numeric,
            name,
        };
        let perfumers: Vec<EntityRef> = p
            .get("perfumers_array")
            .and_then(Value::as_array)
            .map(|all| {
                all.iter()
                    .filter_map(|pf| Some(entity(routes::PERFUMERS_PREFIX, text(pf, "name_url")?, count(pf, "id"), text(pf, "name")?)))
                    .collect()
            })
            .unwrap_or_default();
        // The page of a perfumer is only reached by number: the ones met here are remembered.
        for pf in &perfumers {
            if let Some(pf_id) = pf.numeric_id {
                self.remember_perfumer(pf_id, &pf.id, &pf.name);
            }
        }
        let ratings = SCALES
            .iter()
            .filter_map(|(scale, suffix)| {
                let votes = count(p, &format!("total_votings{suffix}"))?;
                Some(RatingScore {
                    dimension: scale.to_string(),
                    average: out_of_ten(number(p, &format!("avg_voting{suffix}"))?),
                    votes: u32::try_from(votes).ok(),
                })
            })
            .collect();

        // A chart is the one at its own place in `c`, whatever the others are: one that is
        // missing or unreadable must not shift the next ones under its name and its votes.
        let charts = record.get("c").map(by_position).unwrap_or_default();
        let totals = record.get("c_total").map(by_position).unwrap_or_default();
        let votes = |place: usize| at(&totals, place).and_then(|total| total.as_u64().or_else(|| total.as_str()?.trim().parse().ok()));
        let mut warnings = Vec::new();
        let classification: Vec<ClassGroup> = CLASS_GROUPS
            .iter()
            .filter_map(|(place, group)| {
                // A chart nobody voted in is no chart of the web record either: an empty
                // list, or none at all beside a count of no votes. One that is missing
                // where votes are counted, that is not a list, or that holds a slice which
                // cannot be read, is a chart the web record has and this one has not: it is told.
                let shares = match at(&charts, *place).filter(|chart| !chart.is_null()) {
                    None if votes(*place).unwrap_or(0) == 0 => return None,
                    None => None,
                    Some(chart) if !(chart.is_array() || chart.is_object()) => None,
                    Some(chart) if *place == AUDIENCE => Some(pole_shares(chart, record.get("c_poles"), &mut warnings)),
                    Some(chart) => pie_shares(chart),
                };
                let Some(shares) = shares else {
                    let accords = if *group == ACCORDS_GROUP { ", nor the accords, which are its strongest shares" } else { "" };
                    warnings.push(format!("classification: the \"{group}\" chart cannot be read from what the application interface gives{accords}"));
                    return None;
                };
                (!shares.is_empty()).then(|| ClassGroup {
                    group: group.to_string(),
                    votes: votes(*place).and_then(|n| u32::try_from(n).ok()),
                    shares,
                })
            })
            .collect();
        Ok(Ok(PerfumeDetails {
            url: self.t.absolute(&routes::perfume(&id)),
            p_id: Some(p_id),
            name: text(p, "con_ext_name").or_else(|| text(p, "name")).ok_or_else(|| outdated("no name"))?,
            brand: entity(routes::PERFUMES_PREFIX, brand_url, count(b, "id"), text(b, "name").ok_or_else(|| outdated("no brand name"))?),
            year: count(p, "release_year").and_then(|y| u16::try_from(y).ok()),
            gender: gender(p, "gender"),
            concentration: text(p, "con_ext_concentration"),
            perfumers,
            notes: NotePyramid {
                top: note_names(p, "top_notes"),
                heart: note_names(p, "middle_notes"),
                base: note_names(p, "base_notes"),
                flat: note_names(p, "notes"),
            },
            // The main accords of the web page: the strongest shares of the "Type" chart.
            accords: classification
                .iter()
                .find(|g| g.group == ACCORDS_GROUP)
                .map(|g| g.shares.iter().take(MAX_ACCORDS).map(|s| s.label.clone()).collect())
                .unwrap_or_default(),
            ratings,
            classification,
            ranking: None,
            // `info` is the text of the German site: no fact is better than one in another
            // language than the rest of the record.
            facts: plain_text(p, "info_en"),
            description: plain_text(p, "desc"),
            reviews_count: count(p, "total_reviews_en").and_then(|n| u32::try_from(n).ok()),
            statements_count: count(p, "total_statements_en").and_then(|n| u32::try_from(n).ok()),
            image_url: ["url_image_480", "url_image_240"].iter().find_map(|k| text(p, k)),
            id,
            warnings,
        }))
    }

    /// A row of a listing of perfumes: a document of the search index (`GET /perfumes`, and
    /// the listings of a brand or a perfumer), or a row of `GET /search`, which names the
    /// perfume and its concentration apart (`con_ext_name`, `con_ext_concentration`) and
    /// its year and gender without the `p_` of the index. A document of the index only has
    /// the full name ("Sauvage (Eau de Toilette)"): it is kept whole, and no concentration
    /// is guessed out of it.
    ///
    /// Every listing served today (search, brand, perfumer, release year) is made of
    /// documents of the index: all of them give the full name and no concentration. No read
    /// lists the rows of `GET /search`, which is only asked for the brand of a lookup; what
    /// is read of them here is for the day one does.
    pub(crate) fn summary_from_index_row(&self, row: &Value) -> Option<PerfumeSummary> {
        let (brand_url, name_url) = (text(row, "b_name_url")?, text(row, "p_name_url")?);
        let votes = count(row, "p_total_votings");
        if let Some(p_id) = count(row, "p_id") {
            self.remember_perfume(&format!("{brand_url}/{name_url}"), p_id);
        }
        Some(PerfumeSummary {
            id: format!("{brand_url}/{name_url}"),
            p_id: count(row, "p_id"),
            name: text(row, "con_ext_name").or_else(|| text(row, "p_name"))?,
            brand: text(row, "b_name"),
            year: count(row, "p_release_year").or_else(|| count(row, "release_year")).and_then(|y| u16::try_from(y).ok()),
            concentration: text(row, "con_ext_concentration"),
            rating: votes.and_then(|_| number(row, "p_avg_voting")).filter(|r| *r > 0.0).map(out_of_ten),
            rating_count: votes.and_then(|n| u32::try_from(n).ok()),
            gender: gender(row, "p_gender").or_else(|| gender(row, "gender")),
            reviews_count: count(row, "p_total_reviews_en").and_then(|n| u32::try_from(n).ok()),
            url: self.t.absolute(&format!("{}{brand_url}/{name_url}", routes::PERFUMES_PREFIX)),
            image_url: ["url_image_240", "url_image"].iter().find_map(|k| text(row, k)),
            ..PerfumeSummary::default()
        })
    }

    /// The id of a brand from its name, through the search of the application: an exact
    /// name, else the only brand the words match. `None` when the search does not tell
    /// which brand is meant: that is not a brand Parfumo does not have.
    async fn app_brand_id(&self, name: &str) -> Result<Option<u64>, ClientError> {
        let name = name.trim();
        let Some(found) = self.app_read("/search", &[pair("filter", name)]).await? else { return Ok(None) };
        self.refresh_perfumes(&found);
        let brands = found.get("brands").and_then(Value::as_array).cloned().unwrap_or_default();
        let same = |b: &&Value| text(b, "b_name").is_some_and(|n| n.to_lowercase() == name.to_lowercase());
        let hit = brands.iter().find(same).or(if brands.len() == 1 { brands.first() } else { None });
        Ok(hit.and_then(|b| count(b, "b_id")))
    }

    /// The search through the application interface, when the query can be expressed
    /// there: words, one brand, one exact year, genders and a sort order. `None` when the
    /// web search must answer: for the criteria the interface does not know (perfumer,
    /// notes, a range of years), for a brand its search does not find, and for an answer
    /// that does not say how many perfumes match.
    pub(crate) async fn app_search_perfumes(&self, q: &SearchQuery, page: u32) -> Result<Option<Listing<PerfumeSummary>>, ClientError> {
        let filled = |s: &Option<String>| s.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        if filled(&q.perfumer).is_some() || q.notes.iter().any(|n| !n.trim().is_empty()) || q.year_from.is_some() || q.year_to.is_some() {
            return Ok(None);
        }
        let (words, brand) = (filled(&q.text), filled(&q.brand));
        if words.is_none() && brand.is_none() && q.year.is_none() {
            return Err(ClientError::InvalidInput("give at least one of: text, brand, perfumer, notes, year, year_from, year_to".into()));
        }
        let page = page.max(1);
        let mut params = vec![pair("page", page), pair("per_page", PER_PAGE)];
        if let Some(words) = &words {
            params.push(pair("filter", words));
        }
        if let Some(brand) = &brand {
            let Some(b_id) = self.app_brand_id(brand).await? else { return Ok(None) };
            params.push(pair("b_id", b_id));
        }
        if let Some(year) = q.year {
            params.push(pair("y_0", year));
        }
        // Without a gender the listing must be asked for all three.
        let wanted = |g: Gender| q.genders.is_empty() || q.genders.contains(&g);
        for (key, g) in [("g_f", Gender::Women), ("g_m", Gender::Men), ("g_u", Gender::Unisex)] {
            params.push(pair(key, u8::from(wanted(g))));
        }
        // An order is always sent, whatever the criteria: the interface honours every
        // order of the web search (measured), and lists by name when it gets none.
        params.push(pair("o", routes::search_sort(q.sort).unwrap_or(DEFAULT_ORDER)));
        let Some(found) = self.app_read("/perfumes", &params).await? else { return Ok(None) };
        let rows = found.get("rows").and_then(Value::as_array).ok_or_else(|| ClientError::ParserOutdated("/app/v1/perfumes: no rows".into()))?;
        let Some(total) = matches(&found) else { return Ok(None) };
        // A page that starts past the last match holds nothing, whatever the site answers.
        let past_end = u64::from(page - 1) * u64::from(PER_PAGE) >= total;
        Ok(Some(Listing {
            items: if past_end { Vec::new() } else { rows.iter().filter_map(|row| self.summary_from_index_row(row)).collect() },
            page,
            // Every page is promised: the web search stops at 1000 results, the interface
            // serves the pages beyond (measured).
            total_pages: Some(u32::try_from(total.div_ceil(u64::from(PER_PAGE))).unwrap_or(u32::MAX)),
            total_items: Some(u32::try_from(total).unwrap_or(u32::MAX)),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::{Value, json};

    use super::{MAX_PERFUMES, MAX_REFUSAL_CHARS, Unserved, plain, refusal, remember};
    use crate::{ClientConfig, GlobalLimiter, ParfumoClient};

    fn client() -> ParfumoClient {
        ParfumoClient::new(ClientConfig::default(), GlobalLimiter::new(0), None).unwrap()
    }

    fn remembered(c: &ParfumoClient) -> Vec<(u64, String)> {
        let mut all: Vec<(u64, String)> = c.t.perfume_ids.lock().unwrap().values().cloned().collect();
        all.sort();
        all
    }

    #[test]
    fn a_renamed_perfume_is_only_remembered_under_its_new_address() {
        let c = client();
        c.remember_perfume("Creed/Acier_Aluminium", 7);
        c.remember_perfume("Creed/aventus", 9438);
        // The same number comes back under another slug: the site renamed the perfume.
        c.remember_perfume("Creed/acier-aluminium", 7);
        assert_eq!(remembered(&c), [(7, "Creed/acier-aluminium".to_string()), (9438, "Creed/aventus".to_string())]);
        assert!(!c.t.perfume_ids.lock().unwrap().contains_key("creed/acier_aluminium"), "the old address names nothing any more");
    }

    #[test]
    fn the_memory_of_perfumes_is_bounded() {
        let c = client();
        for p_id in 1..=(MAX_PERFUMES as u64 + 10) {
            c.remember_perfume(&format!("Brand/perfume-{p_id}"), p_id);
        }
        let known = c.t.perfume_ids.lock().unwrap();
        assert!(known.len() <= MAX_PERFUMES, "{} perfumes remembered", known.len());
        assert!(known.contains_key(&format!("brand/perfume-{}", MAX_PERFUMES + 10)), "what was met last is known");
    }

    #[test]
    fn a_full_memory_is_emptied_and_a_known_key_is_only_replaced() {
        let mut memory = HashMap::new();
        for n in 0..3 {
            remember(&mut memory, 3, n, n);
        }
        remember(&mut memory, 3, 1, 10);
        assert_eq!((memory.len(), memory[&1]), (3, 10), "a key already there takes no room");
        remember(&mut memory, 3, 4, 4);
        assert_eq!(memory.into_iter().collect::<Vec<_>>(), [(4, 4)]);
    }

    #[test]
    fn the_perfume_looked_for_is_found_in_its_listing_even_when_the_listing_empties_the_memory() {
        let c = client();
        let listing: Value = serde_json::from_str(include_str!("../tests/fixtures/app_perfumes_brand.json")).unwrap();
        let rows = listing["rows"].as_array().unwrap();
        assert_eq!((&rows[2]["p_name_url"], rows.len()), (&json!("absolu-aventus"), 5));
        // Room for the rows up to the one looked for, the third: the fourth empties the memory.
        for p_id in 1..=(MAX_PERFUMES as u64 - 3) {
            c.remember_perfume(&format!("Brand/perfume-{p_id}"), 1_000_000 + p_id);
        }
        assert_eq!(c.perfume_among(rows, "creed/Absolu-Aventus"), Some(225233));
        let known = c.t.perfume_ids.lock().unwrap();
        assert!(!known.contains_key("creed/absolu-aventus") && known.len() == 2, "the memory was emptied after that row: {}", known.len());
        drop(known);
        assert_eq!(c.perfume_among(rows, "Creed/no-such-perfume"), None);
    }

    #[test]
    fn the_search_refreshes_the_address_of_a_perfume_met_before_and_adds_none() {
        let c = client();
        let search: Value = serde_json::from_str(include_str!("../tests/fixtures/app_search_creed.json")).unwrap();
        // The recorded search lists three perfumes of Creed, Aventus (9438) as Creed/aventus.
        assert_eq!(search["perfumes"].as_array().unwrap().len(), 3);
        c.refresh_perfumes(&search);
        assert!(remembered(&c).is_empty(), "a perfume no read met is not remembered from the search");

        c.remember_perfume("Creed/Aventus_Old", 9438);
        c.remember_perfume("Creed/absolu-aventus", 225233);
        c.refresh_perfumes(&search);
        assert_eq!(remembered(&c), [(9438, "Creed/aventus".to_string()), (225233, "Creed/absolu-aventus".to_string())]);
        let known = c.t.perfume_ids.lock().unwrap();
        assert!(known.contains_key("creed/aventus") && !known.contains_key("creed/aventus_old"), "the old address names nothing any more");
    }

    #[test]
    fn a_refusal_is_an_absence_only_when_it_says_not_found() {
        assert_eq!(refusal(&json!({"p": {}, "b": {}})), None);
        assert_eq!(refusal(&json!({"status": "success", "error": null})), None);
        // The refusal recorded for a brand the interface does not have.
        assert_eq!(refusal(&json!({"error": "Brand not found"})), Some(Unserved::Absent));
        assert_eq!(refusal(&json!({"status": "error", "message": "Perfume Not Found"})), Some(Unserved::Absent));
        assert_eq!(refusal(&json!({"status": "error", "message": "Invalid type"})), Some(Unserved::Refused("Invalid type".into())));
        assert_eq!(refusal(&json!({"error": true})), Some(Unserved::Refused(String::new())));
        // Its words end in an error message: on one line, and not a page of them.
        let long = refusal(&json!({"error": format!("Too\n busy {}", "x".repeat(1_000))}));
        let Some(Unserved::Refused(words)) = long else { panic!("{long:?}") };
        assert!(words.starts_with("Too busy x") && words.chars().count() == MAX_REFUSAL_CHARS, "{words}");
    }

    #[test]
    fn a_line_break_is_one_break_with_or_without_the_markup_of_the_site() {
        // The recorded texts: `<br />` followed by the raw break it stands for.
        assert_eq!(plain("One.<br />\r\nTwo.<br />\nThree.<br>Four."), "One.\nTwo.\nThree.\nFour.");
        // Raw breaks alone part lines too, and never glue two words together.
        assert_eq!(plain("One.\r\nTwo.\nThree\rfour."), "One.\nTwo.\nThree\nfour.");
        assert_eq!(plain("One.<br />\r\n<br />\r\n\r\nTwo &amp; <b>three</b>."), "One.\n\nTwo & three.");
    }

    /// The rows of the recorded `GET /search?filter=creed` name the perfume and its
    /// concentration apart; the documents of the index (`GET /perfumes`) do not. No read
    /// lists such rows today: this only pins what the mapper makes of them.
    #[test]
    fn a_row_that_names_the_concentration_apart_gives_the_name_and_the_concentration() {
        let c = client();
        let search: Value = serde_json::from_str(include_str!("../tests/fixtures/app_search_creed.json")).unwrap();
        let mut row = search["perfumes"][0].clone();
        assert_eq!((&row["p_name"], &row["con_ext_name"], &row["con_ext_concentration"]), (&json!("Aventus"), &json!("Aventus"), &json!("")));
        let recorded = c.summary_from_index_row(&row).unwrap();
        assert_eq!((recorded.id.as_str(), recorded.name.as_str(), recorded.concentration), ("Creed/aventus", "Aventus", None));
        assert_eq!((recorded.year, recorded.gender.as_deref(), recorded.p_id), (Some(2010), Some("men"), Some(9438)));

        // None of the three recorded rows has a concentration: the same row, with the label
        // the full name of such a perfume repeats in brackets.
        row["p_name"] = json!("Aventus (Eau de Parfum)");
        row["con_ext_concentration"] = json!("Eau de Parfum");
        let labelled = c.summary_from_index_row(&row).unwrap();
        assert_eq!((labelled.name.as_str(), labelled.concentration.as_deref()), ("Aventus", Some("Eau de Parfum")));

        // A document of the index has the full name alone: it is kept whole.
        let index: Value = serde_json::from_str(include_str!("../tests/fixtures/app_perfumes_brand.json")).unwrap();
        let mut document = index["rows"][0].clone();
        document["p_name"] = json!("2000 Fleurs (Eau de Parfum)");
        let whole = c.summary_from_index_row(&document).unwrap();
        assert_eq!((whole.name.as_str(), whole.concentration), ("2000 Fleurs (Eau de Parfum)", None));
    }
}
