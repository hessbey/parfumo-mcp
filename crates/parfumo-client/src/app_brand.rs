//! The page of a brand through the application interface (`docs/APP_API.md`): the same
//! model as the HTML parser produces, filled from JSON.

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    app_db::{brand_by_slug, count, number, text},
    models::{BrandPage, EntityRef, Listing},
    site::routes,
};

/// Rows asked per page: what the application asks, and the size of a page of the web site.
const PER_PAGE: u32 = 20;

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// What a looked-up reference is remembered under: an address names a brand by its slug
/// alone, any other reference may also be a name or a number, so the two are kept apart.
fn memo_key(address: bool, slug: &str) -> String {
    format!("{}{slug}", if address { "/" } else { "" })
}

impl ParfumoClient {
    fn remember_brand(&self, key: String, number: Option<u64>) {
        self.t.brand_ids.lock().unwrap_or_else(|e| e.into_inner()).insert(key, number);
    }

    /// The number of a brand from a reference, understood as the web code does: a Parfumo
    /// URL or path, a slug or a name; and a number, which is the number itself unless it is
    /// the slug of a brand. `None` when the search of the application does not know the
    /// brand under that slug or name. One call the first time a reference is met, none after.
    async fn app_brand_number(&self, r: &str) -> Result<Option<u64>, ClientError> {
        let (absolute, path) = self.ref_path(r)?;
        let slug = if absolute {
            // Not the address of a brand: the web code words the refusal.
            let Some(slug) = path.strip_prefix(routes::PERFUMES_PREFIX).and_then(|rest| rest.split('/').find(|p| !p.is_empty())) else {
                return Ok(None);
            };
            slug
        } else {
            let name = path.strip_prefix(routes::PERFUMES_PREFIX.trim_start_matches('/')).unwrap_or(&path);
            name.split('/').next().unwrap_or(name).trim()
        };
        if slug.is_empty() {
            return Ok(None);
        }
        let key = memo_key(absolute, slug);
        if let Some(known) = self.t.brand_ids.lock().unwrap_or_else(|e| e.into_inner()).get(&key).copied() {
            return Ok(known);
        }
        let brands = self.app_brands_like(slug).await?;
        // The slug as given, else the one the web code guesses from a name.
        let by_slug = brand_by_slug(&brands, slug).or_else(|| brand_by_slug(&brands, &slug.replace(' ', "_")));
        // An address names a brand by its slug alone; anything else may also be its name.
        let found = if absolute || by_slug.is_some() {
            by_slug
        } else {
            let name = slug.replace('_', " ").to_lowercase();
            let by_name = brands.iter().find(|b| text(b, "b_name").is_some_and(|n| n.to_lowercase() == name)).and_then(|b| count(b, "b_id"));
            // Tried last, so that a brand named by digits alone (`4711`) is still reached by its slug.
            let digits = slug.bytes().all(|c| c.is_ascii_digit());
            by_name.or_else(|| slug.parse().ok().filter(|n| digits && *n > 0))
        };
        self.remember_brand(key, found);
        Ok(found)
    }

    /// The record of a brand and one page of its perfumes (`GET /brand_details`), the most
    /// rated first, as on the web page (measured: without an order they come by name).
    /// `None` when the interface cannot tell which brand is meant: the caller then reads
    /// the web page.
    pub(crate) async fn app_get_brand(&self, r: &str, page: u32) -> Result<Option<BrandPage>, ClientError> {
        let Some(b_id) = self.app_brand_number(r).await? else { return Ok(None) };
        let page = page.max(1);
        // Without a gender the listing must be asked for all three.
        let params = [
            pair("b_id", b_id),
            pair("page", page),
            pair("per_page", PER_PAGE),
            pair("o", "nr_desc"),
            pair("g_f", 1),
            pair("g_m", 1),
            pair("g_u", 1),
        ];
        let answer = self.app_get("/brand_details", &params).await?;
        let outdated = |what: &str| ClientError::ParserOutdated(format!("/app/v1/brand_details: {what}"));
        // No brand in the answer, or another one than asked: a number no brand has. What the
        // interface answers for such a number was not recorded, and a wrong brand must not pass.
        let b = answer.get("b").filter(|b| count(b, "id") == Some(b_id)).ok_or_else(|| ClientError::NotFound(format!("brand {b_id}")))?;
        let slug = text(b, "name_url").ok_or_else(|| outdated("no brand id"))?;
        let name = text(b, "name").ok_or_else(|| outdated("no brand name"))?;
        let listing = answer.get("p").ok_or_else(|| outdated("no perfumes"))?;
        let rows = listing.get("rows").and_then(Value::as_array).ok_or_else(|| outdated("no rows"))?;
        // A count the answer does not carry is not a count of zero.
        let total = number(listing, "matches").filter(|n| *n >= 0.0).map(|n| n as u64);
        // The id handed back is the slug of the site: the next page asked by it needs no search.
        self.remember_brand(memo_key(true, &slug), Some(b_id));
        self.remember_brand(memo_key(false, &slug), Some(b_id));
        Ok(Some(BrandPage {
            brand: EntityRef { url: self.t.absolute(&format!("{}{slug}", routes::PERFUMES_PREFIX)), numeric_id: Some(b_id), name, id: slug },
            // The interface names the country by a number only.
            country: None,
            founded: count(b, "founding_year").and_then(|y| u16::try_from(y).ok()),
            perfumes: Listing {
                items: rows.iter().filter_map(|row| self.summary_from_index_row(row)).collect(),
                page,
                // Neither the application nor the web site lists more rows of one brand.
                total_pages: total.map(|t| t.min(u64::from(routes::MAX_LISTED_RESULTS)).div_ceil(u64::from(PER_PAGE)) as u32),
                total_items: total.and_then(|t| u32::try_from(t).ok()),
            },
        }))
    }
}
