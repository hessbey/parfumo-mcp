//! The page of a perfumer through the application interface (`docs/APP_API.md`): the same
//! model as the HTML parser produces, filled from JSON.

use serde_json::Value;

use crate::{
    ClientError, ParfumoClient,
    app_db::{count, text},
    models::{EntityRef, Listing, PerfumerPage},
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
    /// Remembers the number of a perfumer under their slug and their name: the interface
    /// only knows perfumers by number, and no call of it finds one.
    pub(crate) fn remember_perfumer(&self, pf_id: u64, slug: &str, name: &str) {
        let mut known = self.t.perfumer_ids.lock().unwrap_or_else(|e| e.into_inner());
        // A slug names one perfumer; a name only stands where no slug does.
        known.entry(name.to_lowercase()).or_insert(pf_id);
        known.insert(slug.to_lowercase(), pf_id);
    }

    /// The number of a perfumer from a reference, read as the web code reads it (a Parfumo
    /// URL or path, a slug or a name): the one remembered from a perfume record or from a
    /// page read before, else a bare number, which is the number itself. `None` for any
    /// other reference. Nothing is asked of the site.
    fn app_perfumer_number(&self, r: &str) -> Result<Option<u64>, ClientError> {
        let (absolute, path) = self.ref_path(r)?;
        let named = if absolute {
            // Not the address of a perfumer: the web code words the refusal.
            let Some(slug) = path.strip_prefix(routes::PERFUMERS_PREFIX).and_then(|rest| rest.split('/').find(|p| !p.is_empty())) else {
                return Ok(None);
            };
            slug
        } else {
            let name = path.strip_prefix(routes::PERFUMERS_PREFIX.trim_start_matches('/')).unwrap_or(&path);
            name.split('/').next().unwrap_or(name).trim()
        };
        let known = self.t.perfumer_ids.lock().unwrap_or_else(|e| e.into_inner());
        // As given, as the slug the web code guesses from a name, as the name a slug stands for.
        let met = [named.to_lowercase(), named.replace(' ', "_").to_lowercase(), named.replace('_', " ").to_lowercase()]
            .iter()
            .find_map(|key| known.get(key).copied());
        let digits = !absolute && !path.is_empty() && path.bytes().all(|c| c.is_ascii_digit());
        Ok(met.or_else(|| path.parse().ok().filter(|n| digits && *n > 0)))
    }

    /// The record of a perfumer and one page of their creations (`GET /perfumer_details`),
    /// in the order of the interface: by name, where the web page lists the most rated
    /// first. `None` when the number of the perfumer is not known: the caller then reads
    /// the web page.
    pub(crate) async fn app_get_perfumer(&self, r: &str, page: u32) -> Result<Option<PerfumerPage>, ClientError> {
        let Some(pf_id) = self.app_perfumer_number(r)? else { return Ok(None) };
        let page = page.max(1);
        // Without a gender the listing must be asked for all three.
        let params = [pair("pf_id", pf_id), pair("page", page), pair("per_page", PER_PAGE), pair("g_f", 1), pair("g_m", 1), pair("g_u", 1)];
        let answer = self.app_get("/perfumer_details", &params).await?;
        let outdated = |what: &str| ClientError::ParserOutdated(format!("/app/v1/perfumer_details: {what}"));
        // No perfumer in the answer: a number no perfumer has.
        let pf = answer
            .get("pf")
            .filter(|pf| pf.as_object().is_some_and(|o| !o.is_empty()))
            .ok_or_else(|| ClientError::NotFound(format!("perfumer {pf_id}")))?;
        let (slug, name) =
            (text(pf, "name_url").ok_or_else(|| outdated("no perfumer id"))?, text(pf, "name").ok_or_else(|| outdated("no perfumer name"))?);
        let numeric_id = count(pf, "id");
        if let Some(id) = numeric_id {
            self.remember_perfumer(id, &slug, &name);
        }
        let listing = answer.get("p").ok_or_else(|| outdated("no perfumes"))?;
        let rows = listing.get("rows").and_then(Value::as_array).ok_or_else(|| outdated("no rows"))?;
        let total = count(listing, "matches").unwrap_or(0);
        Ok(Some(PerfumerPage {
            perfumer: EntityRef { url: self.t.absolute(&routes::perfumer(&slug)), id: slug, numeric_id, name },
            // The interface names the country by a number only.
            country: None,
            perfumes: Listing {
                items: rows.iter().filter_map(|row| self.summary_from_index_row(row)).collect(),
                page,
                total_pages: Some(total.min(MAX_ROWS).div_ceil(u64::from(PER_PAGE)) as u32),
                total_items: u32::try_from(total).ok(),
            },
        }))
    }
}
