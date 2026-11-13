//! Database reads through the application interface (`docs/APP_API.md`): the same models
//! as the HTML parsers produce, filled from JSON.

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    models::{Gender, Listing, PerfumeSummary, SearchQuery},
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

impl ParfumoClient {
    /// A row of the filtered listing (`GET /perfumes`, and the listings of a brand, a
    /// perfumer or a note): a document of the search index.
    pub(crate) fn summary_from_index_row(&self, row: &Value) -> Option<PerfumeSummary> {
        let (brand_url, name_url) = (text(row, "b_name_url")?, text(row, "p_name_url")?);
        let votes = count(row, "p_total_votings");
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
