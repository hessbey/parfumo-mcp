//! Database reads through the application interface (`docs/APP_API.md`): the same models
//! as the HTML parsers produce, filled from JSON.

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    models::{ClassGroup, ClassShare, EntityRef, Gender, Listing, NotePyramid, PerfumeDetails, PerfumeSummary, RatingScore, SearchQuery},
    site::routes,
};

/// Rows asked per page of a listing: what the application asks.
const PER_PAGE: u32 = 20;

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

/// The names of a list of notes (`[{id, name, …}]`), without the empty placeholders.
fn note_names(p: &Value, key: &str) -> Vec<String> {
    p.get(key).and_then(Value::as_array).map(|notes| notes.iter().filter_map(|n| text(n, "name")).collect()).unwrap_or_default()
}

/// The four charts of the community classification, in the order the interface sends them.
const CLASS_GROUPS: [&str; 4] = ["Type", "Season", "Occasion", "Audience & Style"];
/// The five rating scales: name, then the suffix of their fields.
const SCALES: [(&str, &str); 5] = [("scent", ""), ("durability", "_dur"), ("sillage", "_sillage"), ("bottle", "_bottle"), ("pricing", "_pricing")];

impl ParfumoClient {
    fn remember_perfume(&self, id: &str, p_id: u64) {
        self.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_lowercase(), p_id);
    }

    /// The number of a perfume from a reference: a number, a Parfumo URL or a
    /// `Brand/Perfume` id. An id met in a listing is known at once; another is looked up
    /// through its brand, in two calls.
    pub(crate) async fn app_perfume_id(&self, reference: &str) -> Result<u64, ClientError> {
        let reference = reference.trim();
        if !reference.is_empty() && reference.chars().all(|c| c.is_ascii_digit()) {
            return reference.parse().map_err(|_| ClientError::InvalidInput("not a perfume number".into()));
        }
        let id = self.perfume_ref_id(reference)?;
        let known = |c: &Self| c.t.perfume_ids.lock().unwrap_or_else(|e| e.into_inner()).get(&id.to_lowercase()).copied();
        if let Some(p_id) = known(self) {
            return Ok(p_id);
        }
        let (brand_url, name_url) = id.split_once('/').ok_or_else(|| ClientError::InvalidInput("not a perfume reference".into()))?;
        let words = |slug: &str| slug.replace(['_', '-'], " ");
        let found = self.app_get("/search", &[pair("filter", words(brand_url))]).await?;
        let brand = found
            .get("brands")
            .and_then(Value::as_array)
            .and_then(|brands| brands.iter().find(|b| text(b, "b_name_url").is_some_and(|u| u.eq_ignore_ascii_case(brand_url))))
            .and_then(|b| count(b, "b_id"))
            .ok_or_else(|| ClientError::NotFound(id.clone()))?;
        let params = [
            pair("page", 1),
            pair("per_page", PER_PAGE),
            pair("filter", words(name_url)),
            pair("b_id", brand),
            pair("g_f", 1),
            pair("g_m", 1),
            pair("g_u", 1),
        ];
        let listing = self.app_get("/perfumes", &params).await?;
        // Reading the rows records their numbers.
        for row in listing.get("rows").and_then(Value::as_array).into_iter().flatten() {
            self.summary_from_index_row(row);
        }
        known(self).ok_or(ClientError::NotFound(id))
    }

    /// The full record of a perfume (`GET /perfume_details/{id}`).
    pub(crate) async fn app_get_perfume(&self, reference: &str) -> Result<PerfumeDetails, ClientError> {
        let p_id = self.app_perfume_id(reference).await?;
        let record = self.app_get(&format!("/perfume_details/{p_id}"), &[]).await?;
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
        let perfumers = p
            .get("perfumers_array")
            .and_then(Value::as_array)
            .map(|all| {
                all.iter()
                    .filter_map(|pf| Some(entity(routes::PERFUMERS_PREFIX, text(pf, "name_url")?, count(pf, "id"), text(pf, "name")?)))
                    .collect()
            })
            .unwrap_or_default();
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
        let charts: Vec<&Vec<Value>> =
            record.get("c").and_then(Value::as_array).map(|c| c.iter().filter_map(Value::as_array).collect()).unwrap_or_default();
        let totals = record.get("c_total").and_then(Value::as_array);
        let classification: Vec<ClassGroup> = charts
            .iter()
            .zip(CLASS_GROUPS)
            .enumerate()
            .filter(|(_, (chart, _))| !chart.is_empty())
            .map(|(i, (chart, group))| {
                let mut shares: Vec<ClassShare> = chart
                    .iter()
                    .filter_map(|s| Some(ClassShare { label: text(s, "n")?, percent: (number(s, "p")? * 10.0).round() as f32 / 10.0 }))
                    .collect();
                shares.sort_by(|a, b| b.percent.total_cmp(&a.percent));
                ClassGroup {
                    group: group.to_string(),
                    votes: totals.and_then(|t| t.get(i)).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
                    shares,
                }
            })
            .collect();
        Ok(PerfumeDetails {
            url: text(p, "url_web").unwrap_or_else(|| self.t.absolute(&format!("{}{id}", routes::PERFUMES_PREFIX))),
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
            // The main accords are the strongest shares of the "Type" chart.
            accords: classification
                .iter()
                .find(|g| g.group == "Type")
                .map(|g| g.shares.iter().map(|s| s.label.clone()).collect())
                .unwrap_or_default(),
            ratings,
            classification,
            ranking: None,
            facts: text(p, "info_en").or_else(|| text(p, "info")),
            description: text(p, "desc"),
            reviews_count: count(p, "total_reviews_en").and_then(|n| u32::try_from(n).ok()),
            statements_count: count(p, "total_statements_en").and_then(|n| u32::try_from(n).ok()),
            image_url: ["url_image_480", "url_image_240"].iter().find_map(|k| text(p, k)),
            id,
            warnings: Vec::new(),
        })
    }

    /// A row of the filtered listing (`GET /perfumes`, and the listings of a brand, a
    /// perfumer or a note): a document of the search index.
    pub(crate) fn summary_from_index_row(&self, row: &Value) -> Option<PerfumeSummary> {
        let (brand_url, name_url) = (text(row, "b_name_url")?, text(row, "p_name_url")?);
        let votes = count(row, "p_total_votings");
        if let Some(p_id) = count(row, "p_id") {
            self.remember_perfume(&format!("{brand_url}/{name_url}"), p_id);
        }
        Some(PerfumeSummary {
            id: format!("{brand_url}/{name_url}"),
            p_id: count(row, "p_id"),
            name: text(row, "p_name")?,
            brand: text(row, "b_name"),
            year: count(row, "p_release_year").and_then(|y| u16::try_from(y).ok()),
            rating: votes.and_then(|_| number(row, "p_avg_voting")).filter(|r| *r > 0.0).map(out_of_ten),
            rating_count: votes.and_then(|n| u32::try_from(n).ok()),
            gender: gender(row, "p_gender"),
            reviews_count: count(row, "p_total_reviews_en").and_then(|n| u32::try_from(n).ok()),
            url: self.t.absolute(&format!("{}{brand_url}/{name_url}", routes::PERFUMES_PREFIX)),
            image_url: ["url_image_240", "url_image"].iter().find_map(|k| text(row, k)),
            ..PerfumeSummary::default()
        })
    }

    /// The id of a brand from its name, through the search of the application.
    async fn app_brand_id(&self, name: &str) -> Result<u64, ClientError> {
        let found = self.app_get("/search", &[pair("filter", name.trim())]).await?;
        let brands = found.get("brands").and_then(Value::as_array).cloned().unwrap_or_default();
        let same =
            |b: &&Value| text(b, "b_name").is_some_and(|n| n.eq_ignore_ascii_case(name.trim()) || n.to_lowercase() == name.trim().to_lowercase());
        // An exact name, else the only brand the words match.
        let hit = brands.iter().find(same).or(if brands.len() == 1 { brands.first() } else { None });
        hit.and_then(|b| count(b, "b_id")).ok_or_else(|| {
            let known: Vec<String> = brands.iter().filter_map(|b| text(b, "b_name")).take(5).collect();
            let hint = if known.is_empty() { String::new() } else { format!(" (close: {})", known.join(", ")) };
            ClientError::InvalidInput(format!("brand '{}' not found on Parfumo{hint}", name.trim()))
        })
    }

    /// The search through the application interface, when the query can be expressed
    /// there: words, one brand, one exact year, genders and a sort order. `None` for the
    /// criteria it does not know (perfumer, notes, a range of years): the caller then
    /// uses the search of the web site.
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
            params.push(pair("b_id", self.app_brand_id(brand).await?));
        }
        if let Some(year) = q.year {
            params.push(pair("y_0", year));
        }
        // Without a gender the listing must be asked for all three.
        let wanted = |g: Gender| q.genders.is_empty() || q.genders.contains(&g);
        for (key, g) in [("g_f", Gender::Women), ("g_m", Gender::Men), ("g_u", Gender::Unisex)] {
            params.push(pair(key, u8::from(wanted(g))));
        }
        // Words alone come back in alphabetical order: the most rated first is a better
        // default for what an assistant looks for.
        match routes::search_sort(q.sort) {
            Some(order) => params.push(pair("o", order)),
            None if words.is_some() => params.push(pair("o", "nr_desc")),
            None => {}
        }
        let found = self.app_get("/perfumes", &params).await?;
        let rows = found.get("rows").and_then(Value::as_array).ok_or_else(|| ClientError::ParserOutdated("/app/v1/perfumes: no rows".into()))?;
        let total = count(&found, "matches").unwrap_or(0);
        Ok(Some(Listing {
            items: rows.iter().filter_map(|row| self.summary_from_index_row(row)).collect(),
            page,
            total_pages: Some(total.div_ceil(u64::from(PER_PAGE)) as u32),
            total_items: Some(total as u32),
        }))
    }
}
