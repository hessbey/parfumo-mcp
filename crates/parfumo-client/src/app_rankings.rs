//! Rankings through the application interface (`docs/APP_API.md`).
//!
//! Only the perfumes of a release year are read there. The other rankings stay on the web
//! pages, because nothing recorded shows that the interface serves the same lists:
//! - the top lists by gender: the application has a "Top 100" (`GET /discover` with
//!   `feature=tops_flops`, `type` = `tops` or `flops`, `gender` = `female`, `male` or
//!   `unisex`, `period` = `overall`, `week`, `month` or a year from 2012). The one answer
//!   recorded, asked with other values, shares no perfume with the first page of the web
//!   list; its rows name the brand without its address, so the `Brand/Perfume` id cannot
//!   be built; and it has 100 rows on a single page where the web list has 500 over five;
//! - the recent additions: `GET /home` only has a selection ("new and remarkable"), on a
//!   single page, without the address of the brand either;
//! - the three trend lists (collections, wishes, wears over 30 days): the application has
//!   no call for them.

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    app_db::matches,
    models::{Listing, PerfumeSummary, RankingKind},
    site::routes,
};

/// Rows asked per page: what the application asks, and the page size of the web listing.
const PER_PAGE: u32 = 20;

/// The order of the web page of a release year: by name (measured: with this order the
/// interface lists the same perfumes in the same order).
const YEAR_ORDER: &str = "n_asc";

fn pair(k: &str, v: impl ToString) -> (String, String) {
    (k.to_string(), v.to_string())
}

impl ParfumoClient {
    /// The rankings the interface serves: the perfumes of a release year (`GET /perfumes`
    /// with `y_0`), in the order of the web page. `None` for every other ranking, for a
    /// page past the 1000th perfume, where the web page stops, and for an answer that is a
    /// refusal or does not say how many perfumes the year has: the caller then reads the
    /// web site.
    pub(crate) async fn app_rankings(&self, kind: &RankingKind, page: u32) -> Result<Option<Listing<PerfumeSummary>>, ClientError> {
        let RankingKind::Year(year) = kind else { return Ok(None) };
        let page = page.max(1);
        if page > routes::MAX_LISTED_RESULTS.div_ceil(PER_PAGE) {
            return Ok(None);
        }
        // The three genders: the page of the application shows them all unless filtered.
        let params = [
            pair("y_0", year),
            pair("page", page),
            pair("per_page", PER_PAGE),
            pair("o", YEAR_ORDER),
            pair("g_f", 1),
            pair("g_m", 1),
            pair("g_u", 1),
        ];
        let Some(found) = self.app_read("/perfumes", &params).await? else { return Ok(None) };
        let rows = found.get("rows").and_then(Value::as_array).ok_or_else(|| ClientError::ParserOutdated("/app/v1/perfumes: no rows".into()))?;
        // A count the answer does not carry is not a count of zero: the web page tells it.
        let Some(total) = matches(&found) else { return Ok(None) };
        // A page that starts past the last perfume holds nothing, whatever the site answers.
        let past_end = u64::from(page - 1) * u64::from(PER_PAGE) >= total;
        Ok(Some(Listing {
            items: if past_end { Vec::new() } else { rows.iter().filter_map(|row| self.summary_from_index_row(row)).collect() },
            page,
            // The web page lists the first 1000 perfumes of a year: the pages promised are its own.
            total_pages: Some(total.min(u64::from(routes::MAX_LISTED_RESULTS)).div_ceil(u64::from(PER_PAGE)) as u32),
            total_items: u32::try_from(total).ok(),
        }))
    }
}
