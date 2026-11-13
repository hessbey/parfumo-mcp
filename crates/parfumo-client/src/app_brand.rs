//! The page of a brand through the application interface (`docs/APP_API.md`): the same
//! model as the HTML parser produces, filled from JSON.

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    app_db::{brand_by_slug, count, text},
    models::{BrandPage, EntityRef, Listing},
    site::routes,
};

/// Rows asked per page: what the application asks, and the size of a page of the web site.
const PER_PAGE: u32 = 20;
/// The application never asks for more rows of one listing, and the web site shows no more.
const MAX_ROWS: u64 = 1000;

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

impl ParfumoClient {
    /// The number of a brand from a reference, understood as the web code does: a Parfumo
    /// URL or path, a slug or a name; and a number, which is the number itself unless it is
    /// the slug of a brand. `None` when the search of the application does not know the
    /// brand under that slug or name. One call, never more.
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
        let brands = self.app_brands_like(slug).await?;
        // The slug as given, else the one the web code guesses from a name.
        let by_slug = brand_by_slug(&brands, slug).or_else(|| brand_by_slug(&brands, &slug.replace(' ', "_")));
        // An address names a brand by its slug alone; anything else may also be its name.
        if absolute || by_slug.is_some() {
            return Ok(by_slug);
        }
        let name = slug.replace('_', " ").to_lowercase();
        let by_name = brands.iter().find(|b| text(b, "b_name").is_some_and(|n| n.to_lowercase() == name)).and_then(|b| count(b, "b_id"));
        // Tried last, so that a brand named by digits alone (`4711`) is still reached by its slug.
        let digits = slug.bytes().all(|c| c.is_ascii_digit());
        Ok(by_name.or_else(|| slug.parse().ok().filter(|n| digits && *n > 0)))
    }

    /// The record of a brand and one page of its perfumes (`GET /brand_details`), in the
    /// order of the interface: by name, where the web page lists the most rated first.
    /// `None` when the interface cannot tell which brand is meant: the caller then reads
    /// the web page.
    pub(crate) async fn app_get_brand(&self, r: &str, page: u32) -> Result<Option<BrandPage>, ClientError> {
        let Some(b_id) = self.app_brand_number(r).await? else { return Ok(None) };
        let page = page.max(1);
        // Without a gender the listing must be asked for all three.
        let params = [pair("b_id", b_id), pair("page", page), pair("per_page", PER_PAGE), pair("g_f", 1), pair("g_m", 1), pair("g_u", 1)];
        let answer = self.app_get("/brand_details", &params).await?;
        let outdated = |what: &str| ClientError::ParserOutdated(format!("/app/v1/brand_details: {what}"));
        // No brand in the answer: a number no brand has.
        let b =
            answer.get("b").filter(|b| b.as_object().is_some_and(|o| !o.is_empty())).ok_or_else(|| ClientError::NotFound(format!("brand {b_id}")))?;
        let slug = text(b, "name_url").ok_or_else(|| outdated("no brand id"))?;
        let listing = answer.get("p").ok_or_else(|| outdated("no perfumes"))?;
        let rows = listing.get("rows").and_then(Value::as_array).ok_or_else(|| outdated("no rows"))?;
        let total = count(listing, "matches").unwrap_or(0);
        Ok(Some(BrandPage {
            brand: EntityRef {
                url: self.t.absolute(&format!("{}{slug}", routes::PERFUMES_PREFIX)),
                numeric_id: count(b, "id"),
                name: text(b, "name").ok_or_else(|| outdated("no brand name"))?,
                id: slug,
            },
            // The interface names the country by a number only.
            country: None,
            founded: count(b, "founding_year").and_then(|y| u16::try_from(y).ok()),
            perfumes: Listing {
                items: rows.iter().filter_map(|row| self.summary_from_index_row(row)).collect(),
                page,
                total_pages: Some(total.min(MAX_ROWS).div_ceil(u64::from(PER_PAGE)) as u32),
                total_items: Some(total as u32),
            },
        }))
    }
}
