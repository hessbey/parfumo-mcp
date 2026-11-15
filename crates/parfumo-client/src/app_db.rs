//! Database reads through the application interface (`docs/APP_API.md`): the same models
//! as the HTML parsers produce, filled from JSON.
//!
//! The interface answers a read only when its answer is as complete as the one of the web
//! site. Whatever it cannot tell (a reference its own lookup does not find, a listing without
//! its count, a refusal in JSON) is left to the web site, which has the last word. Reviews
//! and statements are never read here: the interface gives one rating where the web page
//! gives five, a relative date and no total.

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
    // Line breaks are the `<br />` of the site; the raw ones that follow them are not kept.
    let broken = html.replace(['\r', '\n'], "").replace("<br />", "\n").replace("<br/>", "\n").replace("<br>", "\n");
    let text: String = scraper::Html::parse_fragment(&broken).root_element().text().collect();
    let mut lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines.dedup_by(|a, b| a.is_empty() && b.is_empty());
    lines.join("\n").trim().to_string()
}

/// A text field of a record, cleaned as every text of the site is; `None` when nothing is left.
fn plain_text(v: &Value, key: &str) -> Option<String> {
    text(v, key).map(|t| plain(&t)).filter(|t| !t.is_empty())
}

/// Whether an answer is a refusal in JSON: the interface answers some of them with status
/// 200 (`{"error": "Brand not found"}`, `{"status": "error", …}`).
pub(crate) fn refused(answer: &Value) -> bool {
    let error = answer.get("error").is_some_and(|e| !matches!(e, Value::Null | Value::Bool(false)) && e.as_str() != Some(""));
    error || text(answer, "status").as_deref() == Some("error")
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
/// The five rating scales: name, then the suffix of their fields.
const SCALES: [(&str, &str); 5] = [("scent", ""), ("durability", "_dur"), ("sillage", "_sillage"), ("bottle", "_bottle"), ("pricing", "_pricing")];

/// The shares of a pie chart (Type, Season, Occasion), strongest first, as the web chart
/// gives them: out of the slices listed. The `p` of the interface also counts the slices
/// it does not list, so those of a "Type" chart do not add up to 100.
fn pie_shares(chart: &Value) -> Vec<ClassShare> {
    let mut slices: Vec<(String, f64)> =
        by_position(chart).into_iter().filter_map(|(_, s)| Some((text(s, "n")?, number(s, "p").filter(|p| *p > 0.0)?))).collect();
    let listed: f64 = slices.iter().map(|(_, p)| p).sum();
    slices.sort_by(|a, b| b.1.total_cmp(&a.1));
    slices.into_iter().map(|(label, p)| ClassShare { label, percent: tenth(p / listed * 100.0) }).collect()
}

/// The shares of the "Audience & Style" chart as the web chart gives them: two pairs that
/// each add up to 100 (Masculine and Feminine, Modern and Classic). The interface gives one
/// share over the four poles and leaves the weakest out: its share is what the three
/// others leave of 100. A pair that cannot be rebuilt is left out and named in `warnings`.
fn pole_shares(chart: &Value, poles: Option<&Value>, warnings: &mut Vec<String>) -> Vec<ClassShare> {
    let slices: Vec<(u64, f64)> =
        by_position(chart).into_iter().filter_map(|(_, s)| Some((count(s, "ct")?, number(s, "p").filter(|p| *p >= 0.0)?))).collect();
    if slices.is_empty() {
        return Vec::new();
    }
    let listed = |ct: u64| slices.iter().find(|(pole, _)| *pole == ct).map(|(_, p)| *p);
    let known = || POLE_PAIRS.iter().flatten().map(|(ct, _)| *ct);
    // The share of a pole this code does not know would be taken for the missing one's.
    let trusted = slices.iter().all(|(ct, _)| known().any(|pole| pole == *ct));
    let missing: Vec<u64> = known().filter(|ct| listed(*ct).is_none()).collect();
    let restored = match missing.as_slice() {
        [only] => Some((*only, (100.0 - slices.iter().map(|(_, p)| p).sum::<f64>()).max(0.0))),
        _ => None,
    };
    let share = |ct: u64| listed(ct).or(restored.filter(|(pole, _)| *pole == ct).map(|(_, p)| p)).filter(|_| trusted);
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
            _ => warnings.push(format!(
                "classification: the {first_name}/{second_name} shares of \"Audience & Style\" cannot be worked out from what the application interface gives"
            )),
        }
    }
    shares
}

impl ParfumoClient {
    fn remember_perfume(&self, id: &str, p_id: u64) {
        self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_lowercase(), (p_id, id.to_string()));
    }

    /// A read of the interface, or `None` when it has nothing to say: an address it answers
    /// 404 to, or a refusal in JSON. The caller then leaves the read to the web site.
    pub(crate) async fn app_read(&self, path: &str, params: &[(String, String)]) -> Result<Option<Value>, ClientError> {
        match self.app_get(path, params).await {
            Ok(answer) if refused(&answer) => Ok(None),
            Ok(answer) => Ok(Some(answer)),
            Err(ClientError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The brands the search of the application finds for a slug or a name (`GET /search`):
    /// `{b_id, b_name, b_name_url}` each. None when the search has nothing to say.
    pub(crate) async fn app_brands_like(&self, reference: &str) -> Result<Vec<Value>, ClientError> {
        let found = self.app_read("/search", &[pair("filter", words(reference))]).await?;
        Ok(found.as_ref().and_then(|f| f.get("brands")).and_then(Value::as_array).cloned().unwrap_or_default())
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
        // Reading the rows records their numbers.
        for row in listing.get("rows").and_then(Value::as_array).into_iter().flatten() {
            self.summary_from_index_row(row);
        }
        Ok(known(self))
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
        self.app_perfume_by_number(p_id).await?.ok_or_else(|| ClientError::NotFound(format!("/perfume_details/{p_id}")))
    }

    /// The full record of a perfume, for the read that has the web page behind it. `None`
    /// when the interface does not find the reference, or has nothing under the number it
    /// remembered for it (a perfume merged or removed since): the web page then answers.
    /// A bare number has no address on the web site: the interface alone tells whether it exists.
    pub(crate) async fn app_perfume_record(&self, reference: &str) -> Result<Option<PerfumeDetails>, ClientError> {
        let Some(p_id) = self.app_perfume_number(reference).await? else { return Ok(None) };
        match self.app_perfume_by_number(p_id).await? {
            Some(record) => Ok(Some(record)),
            None if reference.trim().chars().all(|c| c.is_ascii_digit()) => Err(ClientError::NotFound(format!("/perfume_details/{p_id}"))),
            None => {
                self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, (number, _)| *number != p_id);
                Ok(None)
            }
        }
    }

    /// The record of the perfume that bears a number; `None` when the interface has nothing
    /// under it. Filled as the web record is: same scales, same order, same addresses.
    /// Two parts of the web record are not in the answer and stay absent: the rank label
    /// ("Ranked 31 in Men's Perfume") and, for the facts, anything but their English text.
    async fn app_perfume_by_number(&self, p_id: u64) -> Result<Option<PerfumeDetails>, ClientError> {
        let Some(record) = self.app_read(&format!("/perfume_details/{p_id}"), &[]).await? else { return Ok(None) };
        let outdated = |what: &str| ClientError::ParserOutdated(format!("/app/v1/perfume_details: {what}"));
        let (p, b) = (record.get("p").ok_or_else(|| outdated("no perfume"))?, record.get("b").ok_or_else(|| outdated("no brand"))?);
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
        let mut warnings = Vec::new();
        let classification: Vec<ClassGroup> = CLASS_GROUPS
            .iter()
            .filter_map(|(place, group)| {
                let chart = at(&charts, *place)?;
                let shares = if *place == AUDIENCE { pole_shares(chart, record.get("c_poles"), &mut warnings) } else { pie_shares(chart) };
                (!shares.is_empty()).then(|| ClassGroup {
                    group: group.to_string(),
                    votes: at(&totals, *place).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
                    shares,
                })
            })
            .collect();
        Ok(Some(PerfumeDetails {
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
                .find(|g| g.group == "Type")
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
    use serde_json::{Value, json};

    use crate::{ClientConfig, GlobalLimiter, ParfumoClient};

    fn client() -> ParfumoClient {
        ParfumoClient::new(ClientConfig::default(), GlobalLimiter::new(0), None).unwrap()
    }

    /// The rows of the recorded `GET /search?filter=creed` name the perfume and its
    /// concentration apart; the documents of the index (`GET /perfumes`) do not.
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
